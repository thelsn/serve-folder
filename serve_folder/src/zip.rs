use std::path::{Path, PathBuf};
use std::fs;
use std::io;
use std::io::{Write, Read, BufReader, BufWriter};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use rayon::prelude::*;
use tempfile::tempdir;
use walkdir::WalkDir;

use crate::state::ServerState;
use crate::models::ZipProgress;

// Count files in a directory recursively
pub fn count_files_in_directory(dir: &Path) -> usize {
    let mut count = 0;
    
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                count += 1;
            } else if path.is_dir() {
                count += count_files_in_directory(&path);
            }
        }
    }
    
    count
}

// High-performance ZIP archive creation using multiple threads
pub async fn create_zip_archive(
    root_dir: impl AsRef<Path>,
    base_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    operation_id: String,
    state: ServerState,
) -> io::Result<()> {
    // Convert to owned values that can be moved into the closure
    let root_dir = root_dir.as_ref().to_path_buf();
    let base_dir = base_dir.as_ref().to_path_buf();
    let output_path = output_path.as_ref().to_path_buf();
    
    tokio::task::spawn_blocking(move || {
        // Get total files first
        let total_files = match state.get_progress(&operation_id) {
            Some(progress) if progress.total_files > 0 => progress.total_files,
            _ => count_files_in_directory(&base_dir),
        };
        
        // Initialize progress
        state.update_progress(&operation_id, ZipProgress {
            current_file: "Initializing high-performance compression...".to_string(),
            processed_files: 0,
            total_files,
            percentage: 0.0,
        });

        // Create shared progress trackers
        let processed_count = Arc::new(AtomicUsize::new(0));
        let current_file = Arc::new(Mutex::new(String::new()));
        
        // Create temp directory for intermediate files
        let temp_dir = tempdir()?;
        
        // Start progress tracking thread
        let progress_handle = start_progress_tracking(
            operation_id.clone(), 
            state.clone(), 
            processed_count.clone(), 
            current_file.clone(),
            total_files
        );
        
        // Group files by directory for better locality and compression
        let file_groups = collect_files_by_directory(&base_dir, &root_dir)?;
        
        // Get optimal compression level for speed
        let compression = determine_optimal_compression();
        
        // Create temporary ZIP segments in parallel
        let segment_paths: Vec<PathBuf> = process_file_groups_in_parallel(
            &file_groups, 
            &temp_dir.path(),
            &root_dir, 
            compression, 
            processed_count.clone(),
            current_file.clone()
        )?;
        
        // Merge ZIP segments into final archive
        merge_zip_segments(
            segment_paths, 
            &output_path, 
            &operation_id, 
            state.clone()
        )?;
        
        // Signal progress thread to finish and wait for it
        let _ = progress_handle.join();

        // Final update
        state.update_progress(&operation_id, ZipProgress {
            current_file: "ZIP archive complete".to_string(),
            processed_files: total_files,
            total_files,
            percentage: 100.0,
        });
        
        Ok(())
    }).await?
}

// Start a background thread to track and report progress
fn start_progress_tracking(
    operation_id: String,
    state: ServerState,
    processed_count: Arc<AtomicUsize>,
    current_file: Arc<Mutex<String>>,
    total_files: usize
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let update_interval = std::time::Duration::from_millis(100);
        let mut last_processed = 0;
        
        loop {
            let processed = processed_count.load(Ordering::Relaxed);
            
            // Only update if there's a change
            if processed != last_processed {
                let percentage = if total_files > 0 {
                    (processed as f32 / total_files as f32) * 100.0
                } else {
                    0.0
                };
                
                let current = current_file.lock().unwrap().clone();
                
                state.update_progress(&operation_id, ZipProgress {
                    current_file: current,
                    processed_files: processed,
                    total_files,
                    percentage,
                });
                
                last_processed = processed;
            }
            
            // Exit if all files processed
            if processed >= total_files {
                break;
            }
            
            thread::sleep(update_interval);
        }
    })
}

// Collect files grouped by directory to improve compression efficiency
fn collect_files_by_directory(base_dir: &Path, _root_dir: &Path) -> io::Result<Vec<Vec<PathBuf>>> {
    let mut directory_groups: Vec<Vec<PathBuf>> = Vec::new();
    let mut current_dir = PathBuf::new();
    let mut current_group = Vec::new();
    
    // Walk the directory tree
    for entry in WalkDir::new(base_dir).sort_by_file_name().into_iter().filter_map(|e| e.ok()) {
        let path = entry.path().to_path_buf();
        
        if path.is_file() {
            // If we moved to a new directory, start a new group
            let parent = path.parent().unwrap_or(Path::new(""));
            if !current_dir.as_os_str().is_empty() && parent != current_dir {
                if !current_group.is_empty() {
                    directory_groups.push(std::mem::take(&mut current_group));
                }
                current_dir = parent.to_path_buf();
            } else if current_dir.as_os_str().is_empty() {
                current_dir = parent.to_path_buf();
            }
            
            // Add file to current group
            current_group.push(path);
        }
    }
    
    // Add the last group if not empty
    if !current_group.is_empty() {
        directory_groups.push(current_group);
    }
    
    // Balance groups for optimal parallel processing
    balance_file_groups(&mut directory_groups);
    
    Ok(directory_groups)
}

// Balance file groups to ensure efficient parallel processing
fn balance_file_groups(groups: &mut Vec<Vec<PathBuf>>) {
    // Number of desired groups (based on CPU count)
    let target_groups = (num_cpus::get() * 2).max(4);
    
    // If we have too few groups, split larger ones
    if groups.len() < target_groups {
        // Sort groups by size (largest first)
        groups.sort_by(|a, b| b.len().cmp(&a.len()).reverse());
        
        // Calculate how many more groups we need
        let additional_groups_needed = target_groups - groups.len();
        
        // Split the largest groups
        for i in 0..additional_groups_needed.min(groups.len()) {
            let group = &mut groups[i];
            if group.len() > 10 {  // Only split if enough files
                let split_point = group.len() / 2;
                let second_half = group.split_off(split_point);
                groups.push(second_half);
            }
        }
    }
    // If we have too many small groups, combine them
    else if groups.len() > target_groups * 2 {
        // Sort by size (smallest first)
        groups.sort_by(|a, b| a.len().cmp(&b.len()));
        
        // Combine smallest groups until we reach target_groups
        while groups.len() > target_groups {
            if groups.len() >= 2 {
                let group1 = groups.remove(0);
                let group2 = groups.remove(0);
                let mut combined = group1;
                combined.extend(group2);
                groups.push(combined);
            } else {
                break;
            }
        }
    }
}

// Determine the optimal compression level for maximum speed
fn determine_optimal_compression() -> zip::CompressionMethod {
    // Fastest compression method for speed
    zip::CompressionMethod::Deflated
}

// Process file groups in parallel, creating separate ZIP segments
fn process_file_groups_in_parallel(
    file_groups: &[Vec<PathBuf>],
    temp_dir: &Path,
    root_dir: &Path,
    compression: zip::CompressionMethod,
    processed_count: Arc<AtomicUsize>,
    current_file: Arc<Mutex<String>>,
) -> io::Result<Vec<PathBuf>> {
    let options = zip::write::FileOptions::default()
        .compression_method(compression)
        .unix_permissions(0o755);
    
    // Create a segment path for each group
    let segment_paths: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    
    // Process each group in parallel
    file_groups.par_iter().try_for_each(|group| -> io::Result<()> {
        // Create a unique segment file
        let segment_path = temp_dir.join(format!("segment_{}.zip", fastrand::u64(..)));
        
        // Create ZIP writer for this segment
        let file = BufWriter::new(fs::File::create(&segment_path)?);
        let mut zip = zip::ZipWriter::new(file);
        
        // Process each file in this group
        for file_path in group {
            // Calculate relative path
            let rel_path = file_path.strip_prefix(root_dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            
            // Update current file name for progress
            {
                let mut current = current_file.lock().unwrap();
                *current = rel_path.clone();
            }
            
            // Handle directory entries
            if let Some(parent) = file_path.parent() {
                let parent_rel = parent.strip_prefix(root_dir)
                    .unwrap_or(parent)
                    .to_string_lossy();
                
                if !parent_rel.is_empty() {
                    let dir_path = ensure_trailing_slash(&parent_rel);
                    // Only try to add directory if it's not root or already added
                    // This is a simple approach - in a real implementation you'd track added directories
                    if !dir_path.is_empty() && dir_path != "/" {
                        let _ = zip.add_directory(dir_path, options);
                    }
                }
            }
            
            // Add file to ZIP using streaming to reduce memory usage
            zip.start_file(rel_path, options)?;
            
            // Stream file in chunks
            let mut buffer = vec![0; 64 * 1024];  // 64KB buffer
            let mut file = BufReader::new(fs::File::open(file_path)?);
            
            loop {
                let bytes_read = file.read(&mut buffer)?;
                if bytes_read == 0 { break; }
                zip.write_all(&buffer[..bytes_read])?;
            }
            
            // Update progress counter
            processed_count.fetch_add(1, Ordering::Relaxed);
        }
        
        // Finish this segment
        zip.finish()?;
        
        // Add segment path to the list
        segment_paths.lock().unwrap().push(segment_path);
        
        Ok(())
    })?;
    
    Ok(segment_paths.into_inner().unwrap())
}

// Merge multiple ZIP segments into a final archive
fn merge_zip_segments(
    segment_paths: Vec<PathBuf>,
    output_path: &Path,
    operation_id: &str,
    state: ServerState,
) -> io::Result<()> {
    // Update status
    state.update_progress(operation_id, ZipProgress {
        current_file: "Merging ZIP segments...".to_string(),
        processed_files: 0,
        total_files: 0,
        percentage: 95.0,  // Show high percentage since most work is done
    });
    
    // Create the final ZIP file
    let file = BufWriter::new(fs::File::create(output_path)?);
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored); // No need to compress again
    
    // Process multiple segments in a fast streaming approach
    let buffer_size = 1024 * 1024; // 1MB buffer for faster copying
    let mut buffer = vec![0; buffer_size];
    
    for path in segment_paths {
        // Extract files from this segment and add to final ZIP
        let segment_file = fs::File::open(&path)?;
        let mut segment_reader = zip::ZipArchive::new(segment_file)?;
        
        for i in 0..segment_reader.len() {
            let mut segment_entry = segment_reader.by_index(i)?;
            let entry_name = segment_entry.name().to_string();
            
            // Skip directories in the merge phase
            if segment_entry.is_dir() {
                continue;
            }
            
            // Add the file to our final ZIP
            zip.start_file(entry_name, options)?;
            
            // Stream the file data
            loop {
                let bytes_read = segment_entry.read(&mut buffer)?;
                if bytes_read == 0 { break; }
                zip.write_all(&buffer[..bytes_read])?;
            }
        }
        
        // Clean up this segment file
        let _ = fs::remove_file(path);
    }
    
    // Finalize the ZIP
    zip.finish()?;
    
    Ok(())
}

// Helper function to ensure directory paths end with slash
fn ensure_trailing_slash(path: &str) -> String {
    if path.ends_with('/') || path.is_empty() {
        path.to_string()
    } else {
        format!("{}/", path)
    }
}
