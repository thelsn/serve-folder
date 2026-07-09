use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use walkdir::WalkDir;

use crate::models::ZipProgress;
use crate::state::ServerState;

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

pub async fn create_zip_archive(
    root_dir: impl AsRef<Path>,
    base_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    operation_id: String,
    state: ServerState,
) -> io::Result<()> {
    let root_dir = root_dir.as_ref().to_path_buf();
    let base_dir = base_dir.as_ref().to_path_buf();
    let output_path = output_path.as_ref().to_path_buf();

    tokio::task::spawn_blocking(move || {
        let total_files = match state.get_progress(&operation_id) {
            Some(progress) if progress.total_files > 0 => progress.total_files,
            _ => count_files_in_directory(&base_dir),
        };

        state.update_progress(
            &operation_id,
            ZipProgress {
                current_file: "Creating ZIP archive...".to_string(),
                processed_files: 0,
                total_files,
                percentage: 0.0,
            },
        );

        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);

        let file = BufWriter::new(fs::File::create(&output_path)?);
        let mut zip = zip::ZipWriter::new(file);
        let mut added_dirs = HashSet::new();
        let mut processed_files = 0usize;
        let mut buffer = vec![0; 256 * 1024];

        for entry in WalkDir::new(&base_dir)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let rel_path = path
                .strip_prefix(&root_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            if let Some(parent) = path.parent() {
                let parent_rel = parent
                    .strip_prefix(&root_dir)
                    .unwrap_or(parent)
                    .to_string_lossy()
                    .replace('\\', "/");

                if !parent_rel.is_empty() {
                    let dir_path = ensure_trailing_slash(&parent_rel);
                    if added_dirs.insert(dir_path.clone()) {
                        zip.add_directory(&dir_path, options)?;
                    }
                }
            }

            state.update_progress(
                &operation_id,
                ZipProgress {
                    current_file: rel_path.clone(),
                    processed_files,
                    total_files,
                    percentage: if total_files > 0 {
                        (processed_files as f32 / total_files as f32) * 100.0
                    } else {
                        0.0
                    },
                },
            );

            zip.start_file(&rel_path, options)?;

            let mut source = BufReader::new(fs::File::open(path)?);
            loop {
                let bytes_read = source.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                zip.write_all(&buffer[..bytes_read])?;
            }

            processed_files += 1;
        }

        state.update_progress(
            &operation_id,
            ZipProgress {
                current_file: "Finalizing ZIP archive...".to_string(),
                processed_files,
                total_files,
                percentage: if total_files > 0 { 99.0 } else { 100.0 },
            },
        );

        zip.finish()?;

        state.update_progress(
            &operation_id,
            ZipProgress {
                current_file: "ZIP archive complete".to_string(),
                processed_files: total_files,
                total_files,
                percentage: 100.0,
            },
        );

        Ok(())
    })
    .await?
}

fn ensure_trailing_slash(path: &str) -> String {
    if path.ends_with('/') || path.is_empty() {
        path.to_string()
    } else {
        format!("{}/", path)
    }
}
