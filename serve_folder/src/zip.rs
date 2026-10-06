//! Streaming ZIP creation.
//!
//! The archive is sent to the client while it is being built, so a download starts
//! immediately, nothing is staged on disk, and memory stays bounded however many files a
//! folder has. Worker threads open, read and compress files ahead of a single writer that
//! emits entries in order. Opening files is the slow part on Windows (antivirus scans each
//! one), and doing it in parallel makes zipping freshly copied folders ~10x faster.

use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use futures_util::Stream;
use time::{OffsetDateTime, UtcOffset};
use walkdir::{DirEntry, WalkDir};

use crate::models::ZipProgress;
use crate::state::ServerState;

const STORED: u16 = 0;
const DEFLATED: u16 = 8;
const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
const FLAG_UTF8_NAME: u16 = 1 << 11;
const VERSION_DEFAULT: u16 = 20;
const VERSION_ZIP64: u16 = 45;
// "Made by" Unix, so readers take permissions from the external attributes
const VERSION_MADE_BY: u16 = (3 << 8) | 45;
const DIR_ATTRIBUTES: u32 = (0o040755 << 16) | 0x10;
const U32_MAX: u64 = 0xFFFF_FFFF;

// Files up to this size are read and compressed whole by the worker threads
const SMALL_FILE_LIMIT: u64 = 256 * 1024;
// How many entries the workers may get ahead of the writer. Bounds memory to roughly
// LOOKAHEAD * SMALL_FILE_LIMIT, and the number of files held open.
const LOOKAHEAD: usize = 128;
// Size of the chunks handed to the HTTP response, and how many may wait to be sent
const CHUNK_SIZE: usize = 256 * 1024;
const QUEUED_CHUNKS: usize = 8;
// Streamed files at least this big are written with ZIP64 sizes. The margin below 4 GiB
// covers deflate's worst-case growth and files that grow while being read.
const ZIP64_THRESHOLD: u64 = 0xF000_0000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Streams a ZIP of `dir` as it is built, reporting progress under `operation_id`.
pub fn stream_zip_archive(
    dir: PathBuf,
    operation_id: String,
    state: ServerState,
) -> impl Stream<Item = io::Result<Bytes>> + Send + 'static {
    let (tx, rx) = tokio::sync::mpsc::channel(QUEUED_CHUNKS);

    tokio::task::spawn_blocking(move || {
        let progress = Progress {
            operation_id,
            state,
            total_files: AtomicUsize::new(0),
        };
        progress.report("Starting...", 0);

        let out = ChannelWriter {
            tx: tx.clone(),
            buffer: Vec::with_capacity(CHUNK_SIZE),
        };
        // A panic must not end the stream normally, or the browser would save a truncated
        // zip as if it were complete
        let result = panic::catch_unwind(AssertUnwindSafe(|| write_archive(&dir, out, &progress)))
            .unwrap_or_else(|_| Err(io::Error::other("internal error while zipping")));

        match result {
            Ok(processed_files) => progress.finish(processed_files),
            Err(err) => {
                let cancelled = tx.is_closed();
                if !cancelled {
                    eprintln!("Failed to zip {}: {}", dir.display(), err);
                }
                progress.fail(if cancelled { "Download cancelled" } else { "Failed to create ZIP archive" });
                // Ending the body with an error aborts the response, so the browser shows
                // the download as failed instead of keeping a partial file
                let _ = tx.blocking_send(Err(err));
            }
        }
    });

    futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) })
}

struct Progress {
    operation_id: String,
    state: ServerState,
    // Zero until counting finishes; files are counted alongside zipping so the download
    // doesn't have to wait for it
    total_files: AtomicUsize,
}

impl Progress {
    fn report(&self, current_file: &str, processed_files: usize) {
        let total_files = self.total_files.load(Ordering::Relaxed);
        self.state.update_progress(&self.operation_id, ZipProgress {
            current_file: current_file.to_string(),
            processed_files,
            // Files added while zipping can push the count past the total
            total_files: if total_files > 0 { total_files.max(processed_files) } else { 0 },
            percentage: if total_files > 0 {
                (processed_files as f32 / total_files as f32 * 100.0).min(99.0)
            } else {
                0.0
            },
            ..Default::default()
        });
    }

    fn finish(&self, processed_files: usize) {
        self.state.update_progress(&self.operation_id, ZipProgress {
            current_file: "All files sent".to_string(),
            processed_files,
            total_files: processed_files,
            percentage: 100.0,
            done: true,
            failed: false,
        });
    }

    fn fail(&self, message: &str) {
        self.state.update_progress(&self.operation_id, ZipProgress {
            current_file: message.to_string(),
            failed: true,
            ..Default::default()
        });
    }
}

// Hands the archive to the HTTP response in chunks. Blocks while the client is behind,
// and fails once the client has gone away, which stops the zipping.
struct ChannelWriter {
    tx: tokio::sync::mpsc::Sender<io::Result<Bytes>>,
    buffer: Vec<u8>,
}

impl ChannelWriter {
    fn send_buffer(&mut self) -> io::Result<()> {
        let chunk = std::mem::replace(&mut self.buffer, Vec::with_capacity(CHUNK_SIZE));
        self.tx
            .blocking_send(Ok(Bytes::from(chunk)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "download cancelled"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        if self.buffer.len() >= CHUNK_SIZE {
            self.send_buffer()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            self.send_buffer()
        }
    }
}

// --- Walking the folder ---

// Counting and zipping must walk the tree identically so the progress totals line up.
// Symlinked directories are not followed, which also keeps symlink/junction loops out.
fn walk(dir: &Path) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(dir)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
}

// Follows symlinks, so a link to a file is archived with the target's contents
fn is_file(entry: &DirEntry) -> bool {
    entry.file_type().is_file() || (entry.path_is_symlink() && entry.path().is_file())
}

fn zip_name(dir: &Path, entry: &DirEntry) -> String {
    let relative = entry.path().strip_prefix(dir).unwrap_or(entry.path());
    let mut name = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if entry.file_type().is_dir() {
        name.push('/');
    }
    name
}

struct Job {
    path: PathBuf,
    result: SyncSender<Prepared>,
}

// Entries in archive order, as handed from the walker to the writer
enum Queued {
    Dir { name: String, modified: Option<SystemTime> },
    File { name: String, result: Receiver<Prepared> },
}

enum Prepared {
    /// Read whole; `data` is the content compressed with `method`
    Small { method: u16, crc: u32, data: Vec<u8>, size: u64, modified: Option<SystemTime>, attributes: u32 },
    /// Opened with its start already read, for the writer to stream the rest
    Large { file: File, start: Vec<u8>, method: u16, size_hint: u64, modified: Option<SystemTime>, attributes: u32 },
    /// Locked by another program, no permission, ... These are skipped rather than
    /// failing the whole archive.
    Unreadable(io::Error),
}

fn write_archive(dir: &Path, out: impl Write, progress: &Progress) -> io::Result<usize> {
    let workers = thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 16);
    let stop_counting = AtomicBool::new(false);
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let job_rx = Mutex::new(job_rx);
    let (queue_tx, queue_rx) = mpsc::sync_channel::<Queued>(LOOKAHEAD);

    thread::scope(|scope| {
        scope.spawn(|| {
            let mut count = 0;
            for entry in walk(dir) {
                if stop_counting.load(Ordering::Relaxed) {
                    return;
                }
                if is_file(&entry) {
                    count += 1;
                }
            }
            progress.total_files.store(count, Ordering::Relaxed);
        });

        for _ in 0..workers {
            scope.spawn(|| run_worker(&job_rx));
        }
        scope.spawn(move || run_walker(dir, job_tx, queue_tx));

        // Returning drops the queue, which stops the walker and, through it, the workers
        let result = write_entries(queue_rx, out, progress);
        stop_counting.store(true, Ordering::Relaxed);
        result
    })
}

fn run_walker(dir: &Path, jobs: mpsc::Sender<Job>, queue: SyncSender<Queued>) {
    for entry in walk(dir) {
        let name = zip_name(dir, &entry);
        let queued = if entry.file_type().is_dir() {
            let modified = entry.metadata().ok().and_then(|meta| meta.modified().ok());
            Queued::Dir { name, modified }
        } else if is_file(&entry) {
            let (result_tx, result_rx) = mpsc::sync_channel(1);
            if jobs.send(Job { path: entry.into_path(), result: result_tx }).is_err() {
                return;
            }
            Queued::File { name, result: result_rx }
        } else {
            continue;
        };
        // Blocks while the writer is LOOKAHEAD entries behind; fails once it has stopped
        if queue.send(queued).is_err() {
            return;
        }
    }
}

fn run_worker(jobs: &Mutex<Receiver<Job>>) {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    loop {
        let job = match jobs.lock().unwrap().recv() {
            Ok(job) => job,
            Err(_) => return,
        };
        let prepared = prepare_file(&job.path, &mut encoder).unwrap_or_else(Prepared::Unreadable);
        // The writer may have stopped (download cancelled); nothing to do then
        let _ = job.result.send(prepared);
    }
}

fn prepare_file(path: &Path, encoder: &mut DeflateEncoder<Vec<u8>>) -> io::Result<Prepared> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let modified = metadata.modified().ok();
    let attributes = file_attributes(&metadata);

    // Read the whole file if it's small, otherwise enough to judge how well it compresses
    let mut start = Vec::with_capacity(metadata.len().min(SMALL_FILE_LIMIT + 1) as usize);
    (&mut file).take(SMALL_FILE_LIMIT + 1).read_to_end(&mut start)?;
    encoder.write_all(&start)?;
    let deflated = encoder.reset(Vec::new())?;

    if start.len() as u64 <= SMALL_FILE_LIMIT {
        let crc = crc32fast::hash(&start);
        let size = start.len() as u64;
        let (method, data) = if deflated.len() < start.len() { (DEFLATED, deflated) } else { (STORED, start) };
        Ok(Prepared::Small { method, crc, data, size, modified, attributes })
    } else {
        // Photos, video and archives barely shrink, and storing them is far faster
        let method = if deflated.len() < start.len() / 10 * 9 { DEFLATED } else { STORED };
        Ok(Prepared::Large { file, start, method, size_hint: metadata.len(), modified, attributes })
    }
}

fn write_entries(queue: Receiver<Queued>, out: impl Write, progress: &Progress) -> io::Result<usize> {
    let local_offset = UtcOffset::current_local_offset().ok();
    let mut zip = ZipWriter::new(out);
    let mut processed_files = 0;
    let mut last_report = Instant::now();

    for queued in queue {
        match queued {
            Queued::Dir { name, modified } => {
                if name.len() > u16::MAX as usize {
                    eprintln!("Skipping {}: path too long for a ZIP", name);
                    continue;
                }
                zip.add_buffered(name, STORED, 0, &[], 0, dos_time(modified, local_offset), DIR_ATTRIBUTES)?;
            }
            Queued::File { name, result } => {
                processed_files += 1;
                let prepared = result
                    .recv()
                    .map_err(|_| io::Error::other("a file reader stopped unexpectedly"))?;

                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    progress.report(&name, processed_files);
                    last_report = Instant::now();
                }
                if name.len() > u16::MAX as usize {
                    eprintln!("Skipping {}: path too long for a ZIP", name);
                    continue;
                }

                match prepared {
                    Prepared::Small { method, crc, data, size, modified, attributes } => {
                        zip.add_buffered(name, method, crc, &data, size, dos_time(modified, local_offset), attributes)?
                    }
                    Prepared::Large { mut file, start, method, size_hint, modified, attributes } => zip.add_streamed(
                        name,
                        method,
                        &start,
                        &mut file,
                        size_hint,
                        dos_time(modified, local_offset),
                        attributes,
                    )?,
                    Prepared::Unreadable(err) => eprintln!("Skipping {}: {}", name, err),
                }
            }
        }
    }

    zip.finish()?;
    Ok(processed_files)
}

fn file_attributes(metadata: &Metadata) -> u32 {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let mode = if metadata.permissions().readonly() { 0o444 } else { 0o644 };
    (0o100000 | mode) << 16
}

// ZIP timestamps are MS-DOS local time with 2-second resolution, covering 1980-2107.
// Returns (time, date).
fn dos_time(modified: Option<SystemTime>, local_offset: Option<UtcOffset>) -> (u16, u16) {
    const EARLIEST: (u16, u16) = (0, (1 << 5) | 1); // 1980-01-01 00:00:00
    const LATEST: (u16, u16) = (0xBF7D, 0xFF9F); // 2107-12-31 23:59:58

    // Checked conversions throughout: files can carry nonsense timestamps
    let seconds = match modified.unwrap_or_else(SystemTime::now).duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since_epoch) => i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX),
        Err(_) => return EARLIEST,
    };
    let Ok(utc) = OffsetDateTime::from_unix_timestamp(seconds) else {
        return LATEST;
    };
    let local = match local_offset {
        Some(offset) if utc.year() < 9000 => utc.to_offset(offset),
        _ => utc,
    };

    match local.year() {
        ..=1979 => EARLIEST,
        2108.. => LATEST,
        year => (
            (local.hour() as u16) << 11 | (local.minute() as u16) << 5 | (local.second() as u16 / 2),
            ((year - 1980) as u16) << 9 | (u8::from(local.month()) as u16) << 5 | local.day() as u16,
        ),
    }
}

// --- ZIP container format (PKWARE APPNOTE 6.3) ---

fn put16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn put32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn put64(buf: &mut Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

struct EntryHeader {
    name: String,
    method: u16,
    flags: u16,
    time: (u16, u16),
    attributes: u32,
    crc: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    // Sizes are in ZIP64 form (local header and data descriptor)
    zip64: bool,
    offset: u64,
}

impl EntryHeader {
    fn local_header(&self) -> Vec<u8> {
        let streamed = self.flags & FLAG_DATA_DESCRIPTOR != 0;
        let mut h = Vec::with_capacity(30 + self.name.len() + 20);
        put32(&mut h, 0x0403_4b50);
        put16(&mut h, if self.zip64 { VERSION_ZIP64 } else { VERSION_DEFAULT });
        put16(&mut h, self.flags);
        put16(&mut h, self.method);
        put16(&mut h, self.time.0);
        put16(&mut h, self.time.1);
        // A streamed entry's CRC and sizes come after its data, in the data descriptor
        put32(&mut h, if streamed { 0 } else { self.crc });
        if self.zip64 {
            put32(&mut h, U32_MAX as u32);
            put32(&mut h, U32_MAX as u32);
        } else if streamed {
            put32(&mut h, 0);
            put32(&mut h, 0);
        } else {
            put32(&mut h, self.compressed_size as u32);
            put32(&mut h, self.uncompressed_size as u32);
        }
        put16(&mut h, self.name.len() as u16);
        put16(&mut h, if self.zip64 { 20 } else { 0 });
        h.extend_from_slice(self.name.as_bytes());
        if self.zip64 {
            // ZIP64 extra field, telling readers the data descriptor has 8-byte sizes
            put16(&mut h, 0x0001);
            put16(&mut h, 16);
            put64(&mut h, 0);
            put64(&mut h, 0);
        }
        h
    }

    fn data_descriptor(&self) -> Vec<u8> {
        let mut d = Vec::with_capacity(24);
        put32(&mut d, 0x0807_4b50);
        put32(&mut d, self.crc);
        if self.zip64 {
            put64(&mut d, self.compressed_size);
            put64(&mut d, self.uncompressed_size);
        } else {
            put32(&mut d, self.compressed_size as u32);
            put32(&mut d, self.uncompressed_size as u32);
        }
        d
    }

    fn central_header(&self, out: &mut Vec<u8>) {
        // Values that don't fit in 32 bits go in a ZIP64 extra field, as do the sizes of
        // entries whose local header used ZIP64
        let sizes64 = self.zip64 || self.compressed_size >= U32_MAX || self.uncompressed_size >= U32_MAX;
        let offset64 = self.offset >= U32_MAX;
        let mut extra = Vec::new();
        if sizes64 {
            put64(&mut extra, self.uncompressed_size);
            put64(&mut extra, self.compressed_size);
        }
        if offset64 {
            put64(&mut extra, self.offset);
        }

        put32(out, 0x0201_4b50);
        put16(out, VERSION_MADE_BY);
        put16(out, if sizes64 || offset64 { VERSION_ZIP64 } else { VERSION_DEFAULT });
        put16(out, self.flags);
        put16(out, self.method);
        put16(out, self.time.0);
        put16(out, self.time.1);
        put32(out, self.crc);
        if sizes64 {
            put32(out, U32_MAX as u32);
            put32(out, U32_MAX as u32);
        } else {
            put32(out, self.compressed_size as u32);
            put32(out, self.uncompressed_size as u32);
        }
        put16(out, self.name.len() as u16);
        put16(out, if extra.is_empty() { 0 } else { extra.len() as u16 + 4 });
        put16(out, 0); // comment length
        put16(out, 0); // disk number
        put16(out, 0); // internal attributes
        put32(out, self.attributes);
        put32(out, if offset64 { U32_MAX as u32 } else { self.offset as u32 });
        out.extend_from_slice(self.name.as_bytes());
        if !extra.is_empty() {
            put16(out, 0x0001);
            put16(out, extra.len() as u16);
            out.extend_from_slice(&extra);
        }
    }
}

// Tracks the archive offset while the deflate encoder writes through it
struct CountingWriter<'a, W> {
    out: &'a mut W,
    count: &'a mut u64,
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.out.write(buf)?;
        *self.count += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

// Writes `start` and then the rest of `reader` to `out`, returning their CRC-32 and length
fn copy_with_crc(start: &[u8], reader: &mut impl Read, out: &mut impl Write, buffer: &mut [u8]) -> io::Result<(u32, u64)> {
    let mut crc = crc32fast::Hasher::new();
    crc.update(start);
    out.write_all(start)?;
    let mut length = start.len() as u64;
    loop {
        let read = match reader.read(buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        crc.update(&buffer[..read]);
        out.write_all(&buffer[..read])?;
        length += read as u64;
    }
    Ok((crc.finalize(), length))
}

/// Writes a ZIP archive front to back without seeking, so it can go straight to a socket.
struct ZipWriter<W: Write> {
    out: W,
    offset: u64,
    entries: u64,
    // Kept in memory until the end: about 100 bytes per entry
    central_directory: Vec<u8>,
    buffer: Vec<u8>,
}

impl<W: Write> ZipWriter<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            offset: 0,
            entries: 0,
            central_directory: Vec::new(),
            buffer: vec![0; CHUNK_SIZE],
        }
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    /// Adds an entry whose (possibly compressed) data is already in memory
    #[allow(clippy::too_many_arguments)]
    fn add_buffered(
        &mut self,
        name: String,
        method: u16,
        crc: u32,
        data: &[u8],
        uncompressed_size: u64,
        time: (u16, u16),
        attributes: u32,
    ) -> io::Result<()> {
        let header = EntryHeader {
            name,
            method,
            flags: FLAG_UTF8_NAME,
            time,
            attributes,
            crc,
            compressed_size: data.len() as u64,
            uncompressed_size,
            zip64: false,
            offset: self.offset,
        };
        self.write(&header.local_header())?;
        self.write(data)?;
        header.central_header(&mut self.central_directory);
        self.entries += 1;
        Ok(())
    }

    /// Adds an entry by streaming `start` followed by the rest of `reader`. The CRC and
    /// sizes aren't known until the end, so they follow the data in a data descriptor.
    #[allow(clippy::too_many_arguments)]
    fn add_streamed(
        &mut self,
        name: String,
        method: u16,
        start: &[u8],
        reader: &mut impl Read,
        size_hint: u64,
        time: (u16, u16),
        attributes: u32,
    ) -> io::Result<()> {
        let mut header = EntryHeader {
            name,
            method,
            flags: FLAG_UTF8_NAME | FLAG_DATA_DESCRIPTOR,
            time,
            attributes,
            crc: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            zip64: size_hint >= ZIP64_THRESHOLD,
            offset: self.offset,
        };
        self.write(&header.local_header())?;

        let data_start = self.offset;
        let mut counted = CountingWriter { out: &mut self.out, count: &mut self.offset };
        let (crc, uncompressed_size) = if method == DEFLATED {
            let mut encoder = DeflateEncoder::new(&mut counted, Compression::default());
            let result = copy_with_crc(start, reader, &mut encoder, &mut self.buffer)?;
            encoder.finish()?;
            result
        } else {
            copy_with_crc(start, reader, &mut counted, &mut self.buffer)?
        };

        header.crc = crc;
        header.uncompressed_size = uncompressed_size;
        header.compressed_size = self.offset - data_start;
        if !header.zip64 && (header.compressed_size >= U32_MAX || header.uncompressed_size >= U32_MAX) {
            return Err(io::Error::other(format!("{} grew past 4 GiB while being zipped", header.name)));
        }

        self.write(&header.data_descriptor())?;
        header.central_header(&mut self.central_directory);
        self.entries += 1;
        Ok(())
    }

    fn finish(mut self) -> io::Result<W> {
        let central_directory_offset = self.offset;
        let central_directory = std::mem::take(&mut self.central_directory);
        self.write(&central_directory)?;
        let central_directory_size = central_directory.len() as u64;
        let entries = self.entries;

        let mut end = Vec::with_capacity(98);
        if entries >= 0xFFFF || central_directory_size >= U32_MAX || central_directory_offset >= U32_MAX {
            // ZIP64 end of central directory record, then its locator
            let zip64_end_offset = self.offset;
            put32(&mut end, 0x0606_4b50);
            put64(&mut end, 44); // size of the rest of this record
            put16(&mut end, VERSION_MADE_BY);
            put16(&mut end, VERSION_ZIP64);
            put32(&mut end, 0); // this disk
            put32(&mut end, 0); // disk with the central directory
            put64(&mut end, entries);
            put64(&mut end, entries);
            put64(&mut end, central_directory_size);
            put64(&mut end, central_directory_offset);

            put32(&mut end, 0x0706_4b50);
            put32(&mut end, 0); // disk with the ZIP64 end record
            put64(&mut end, zip64_end_offset);
            put32(&mut end, 1); // total disks
        }
        put32(&mut end, 0x0605_4b50);
        put16(&mut end, 0); // this disk
        put16(&mut end, 0); // disk with the central directory
        put16(&mut end, entries.min(0xFFFF) as u16);
        put16(&mut end, entries.min(0xFFFF) as u16);
        put32(&mut end, central_directory_size.min(U32_MAX) as u32);
        put32(&mut end, central_directory_offset.min(U32_MAX) as u32);
        put16(&mut end, 0); // comment length
        self.write(&end)?;

        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;

    // `::zip` is the zip crate (a dev-dependency), used as an independent reader
    type Reader = ::zip::ZipArchive<Cursor<Vec<u8>>>;

    fn zip_folder(dir: &Path, out: impl Write) -> io::Result<usize> {
        let progress = Progress {
            operation_id: "test".to_string(),
            state: ServerState::new(dir.to_path_buf()),
            total_files: AtomicUsize::new(0),
        };
        write_archive(dir, out, &progress)
    }

    fn read_entry(reader: &mut Reader, name: &str) -> Vec<u8> {
        let mut entry = reader.by_name(name).unwrap_or_else(|_| panic!("{name} is missing"));
        let mut content = Vec::new();
        // Fails on a CRC mismatch
        entry.read_to_end(&mut content).unwrap();
        content
    }

    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn round_trips_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        let files: Vec<(&str, Vec<u8>, ::zip::CompressionMethod)> = vec![
            ("text.txt", b"hello ".repeat(500), ::zip::CompressionMethod::Deflated),
            ("noise.bin", noise(10_000, 1), ::zip::CompressionMethod::Stored),
            ("empty.txt", Vec::new(), ::zip::CompressionMethod::Stored),
            ("sub/deeper/big.log", b"a line of a big, compressible log\n".repeat(100_000), ::zip::CompressionMethod::Deflated),
            ("sub/big noise.bin", noise(3_000_000, 2), ::zip::CompressionMethod::Stored),
            ("sub/日本語 📷.txt", "unicode".as_bytes().to_vec(), ::zip::CompressionMethod::Stored),
        ];
        for (name, data, _) in &files {
            fs::write(root.join(name), data).unwrap();
        }

        let mut archive = Vec::new();
        assert_eq!(zip_folder(root, &mut archive).unwrap(), files.len());
        let mut reader = Reader::new(Cursor::new(archive)).unwrap();

        assert_eq!(reader.len(), files.len() + 3);
        for (name, data, method) in &files {
            assert_eq!(&read_entry(&mut reader, name), data, "{name}");
            assert_eq!(reader.by_name(name).unwrap().compression(), *method, "{name}");
        }
        for name in ["empty/", "sub/", "sub/deeper/"] {
            assert!(reader.by_name(name).unwrap().is_dir(), "{name}");
        }
    }

    #[test]
    fn writes_more_than_65535_entries() {
        let mut zip = ZipWriter::new(Vec::new());
        for i in 0..70_000 {
            zip.add_buffered(format!("file{i}"), STORED, 0, &[], 0, (0, 33), 0o100644 << 16).unwrap();
        }
        let mut reader = Reader::new(Cursor::new(zip.finish().unwrap())).unwrap();
        assert_eq!(reader.len(), 70_000);
        assert_eq!(reader.by_index(69_999).unwrap().name(), "file69999");
    }

    #[test]
    fn streams_entries_with_zip64_sizes() {
        let data = noise(100_000, 3);
        let mut zip = ZipWriter::new(Vec::new());
        // A size hint over the threshold makes the entry ZIP64 without needing 4 GiB of data
        zip.add_streamed("big.bin".to_string(), DEFLATED, &data[..10], &mut &data[10..], ZIP64_THRESHOLD, (0, 33), 0o100644 << 16)
            .unwrap();
        zip.add_buffered("after.txt".to_string(), STORED, crc32fast::hash(b"x"), b"x", 1, (0, 33), 0o100644 << 16)
            .unwrap();
        let mut reader = Reader::new(Cursor::new(zip.finish().unwrap())).unwrap();
        assert_eq!(read_entry(&mut reader, "big.bin"), data);
        assert_eq!(read_entry(&mut reader, "after.txt"), b"x");
    }

    #[test]
    fn dos_time_handles_any_timestamp() {
        let utc = Some(UtcOffset::UTC);
        // 2021-03-04 05:06:07 UTC
        let normal = SystemTime::UNIX_EPOCH + Duration::from_secs(1_614_834_367);
        assert_eq!(dos_time(Some(normal), utc), (5 << 11 | 6 << 5 | 3, 41 << 9 | 3 << 5 | 4));
        assert_eq!(dos_time(Some(SystemTime::UNIX_EPOCH), utc), (0, 33));
        // Around the year 30,000
        let far_future = SystemTime::UNIX_EPOCH + Duration::from_secs(900_000_000_000);
        assert_eq!(dos_time(Some(far_future), utc), (0xBF7D, 0xFF9F));
    }

    struct DisconnectingClient {
        remaining: usize,
    }

    impl Write for DisconnectingClient {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.len() > self.remaining {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "client went away"));
            }
            self.remaining -= buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stops_when_the_client_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        // More files than LOOKAHEAD, so the walker is blocked on a full queue when the
        // writer fails; it must still shut down rather than deadlock
        for i in 0..400 {
            fs::write(dir.path().join(format!("{i}.bin")), noise(2_000, i)).unwrap();
        }
        let err = zip_folder(dir.path(), DisconnectingClient { remaining: 20_000 }).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[cfg(windows)]
    #[test]
    fn skips_files_locked_by_other_programs() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        for name in ["a.txt", "locked.txt", "z.txt"] {
            fs::write(dir.path().join(name), name).unwrap();
        }
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dir.path().join("locked.txt"))
            .unwrap();

        let mut archive = Vec::new();
        zip_folder(dir.path(), &mut archive).unwrap();
        let mut reader = Reader::new(Cursor::new(archive)).unwrap();
        assert_eq!(reader.len(), 2);
        assert_eq!(read_entry(&mut reader, "a.txt"), b"a.txt");
        assert_eq!(read_entry(&mut reader, "z.txt"), b"z.txt");
    }
}
