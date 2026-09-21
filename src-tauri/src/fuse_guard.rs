//! A single-byte, journalled Electron fuse lease. Never replace a running image or
//! overwrite an updated executable. The helper outlives the plugin and restores
//! the byte only after all target processes exit and an exclusive open succeeds.
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::managed_launch::snapshot_executable_processes_strict as snapshot_executable_processes;

pub const TARGET: &str = r"D:\grok_bot\Grok Bot\Grok Bot.exe";
const SENTINEL: &[u8] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";
const INSPECT_INDEX: usize = 3;
const HELPER_ARG: &str = "--fuse-recovery-helper";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

fn io_error(error: impl std::fmt::Display) -> String {
    format!("Grok 临时安全开关保护：{error}")
}

fn root() -> PathBuf {
    crate::worker::data_directory().join("fuse-recovery")
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn inspect_offset(bytes: &[u8]) -> Result<usize, String> {
    let mut matches = bytes
        .windows(SENTINEL.len())
        .enumerate()
        .filter_map(|(offset, value)| (value == SENTINEL).then_some(offset));
    let start = matches
        .next()
        .ok_or_else(|| io_error("找不到 Electron fuse，未修改程序。"))?;
    if matches.next().is_some() {
        return Err(io_error("Electron fuse 标记不唯一，未修改程序。"));
    }
    let version = start + SENTINEL.len();
    let length = usize::from(
        *bytes
            .get(version + 1)
            .ok_or_else(|| io_error("fuse 数据不完整"))?,
    );
    if bytes.get(version) != Some(&1)
        || length <= INSPECT_INDEX
        || bytes.get(version + 2..version + 2 + length).is_none()
    {
        return Err(io_error("不支持的 Electron fuse 格式，未修改程序。"));
    }
    let offset = version + 2 + INSPECT_INDEX;
    if !matches!(bytes[offset], b'0' | b'1') {
        return Err(io_error("Inspector fuse 不可修改，未修改程序。"));
    }
    Ok(offset)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    executable: String,
    offset: usize,
    original_value: u8,
    original_sha256: String,
    patched_sha256: String,
    inspector_port: u16,
    nonce: String,
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn validate_backup(journal: &Journal, executable: &Path, bytes: &mut [u8]) -> Result<(), String> {
    if journal.schema != 1
        || normalized(Path::new(&journal.executable)) != normalized(executable)
        || journal.inspector_port == 0
        || uuid::Uuid::parse_str(&journal.nonce).is_err()
        || hash(bytes) != journal.original_sha256
        || inspect_offset(bytes)? != journal.offset
        || !matches!(journal.original_value, b'0' | b'1')
        || bytes[journal.offset] != journal.original_value
    {
        return Err(io_error(
            "恢复记录或原始备份校验失败；保留备份，拒绝修改程序。",
        ));
    }
    bytes[journal.offset] = b'1';
    let valid = hash(bytes) == journal.patched_sha256;
    bytes[journal.offset] = journal.original_value;
    if !valid {
        return Err(io_error("恢复记录不是合法的单字节 Inspector 补丁。"));
    }
    Ok(())
}

fn exclusive(path: &Path, create: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .share_mode(0)
        .open(path)
}

fn lock(directory: &Path) -> Result<File, String> {
    fs::create_dir_all(directory).map_err(io_error)?;
    // The file persists, the OS-held exclusive handle is the lock. A crash releases it.
    exclusive(&directory.join("transaction.lock"), true).map_err(|error| {
        io_error(format!(
            "另一个启动/恢复操作正在进行，或无法访问恢复目录：{error}"
        ))
    })
}

fn wait_for_lock(directory: &Path) -> Result<File, String> {
    for _ in 0..100 {
        if let Ok(value) = lock(directory) {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(io_error("等待其他启动/恢复事务超时，未启动 Grok。"))
}

fn wait_for_exit() -> Result<(), String> {
    let mut last_error = io_error("Grok 仍有运行进程，等待完全退出。");
    for _ in 0..75 {
        match snapshot_executable_processes(TARGET) {
            Ok(processes) if processes.is_empty() => return Ok(()),
            Ok(_) => {}
            // A just-exited Electron child can remain in a snapshot but no
            // longer support identity queries. Retry, NEVER treat it as absent.
            Err(error) => last_error = error,
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(last_error)
}

fn bytes_from(file: &mut File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

fn write_byte(file: &mut File, offset: usize, value: u8) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(io_error)?;
    file.write_all(&[value])
        .and_then(|_| file.sync_all())
        .map_err(io_error)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(io_error)
}

fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Do not replace an existing record. Publish flushed content with write-through semantics.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

// Immutable journal: publish only after the complete backup and record are flushed.
// No truncate/rewrite of the only recovery record at any point.
fn prepare_record(directory: &Path, journal: &Journal, original: &[u8]) -> Result<(), String> {
    let backup = directory.join("original.exe");
    if backup.exists() {
        if hash(&fs::read(&backup).map_err(io_error)?) != journal.original_sha256 {
            // No pending record exists here: a previous cleanup may have been
            // interrupted. Preserve the orphan, never use it for this version.
            publish(
                &backup,
                &directory.join(format!("orphan-{}.exe", uuid::Uuid::new_v4())),
            )?;
        }
    }
    if !backup.exists() {
        let temporary = directory.join("original.tmp");
        if temporary.exists() {
            fs::remove_file(&temporary).map_err(io_error)?;
        }
        write_new(&temporary, original)?;
        publish(&temporary, &backup)?;
    }
    let temporary = directory.join("pending.tmp");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(io_error)?;
    }
    write_new(
        &temporary,
        &serde_json::to_vec_pretty(journal).map_err(io_error)?,
    )?;
    publish(&temporary, &directory.join("pending.json"))
}

fn clear_record(directory: &Path) -> Result<(), String> {
    // Deleting the record first is safe: this is called only once the EXE is verified original.
    fs::remove_file(directory.join("pending.json")).map_err(io_error)?;
    fs::remove_file(directory.join("original.exe")).map_err(io_error)?;
    let _ = fs::remove_file(directory.join("recovery-error.txt"));
    Ok(())
}

fn restore_locked(directory: &Path, executable: &Path) -> Result<(), String> {
    if !directory.join("pending.json").exists() {
        return Ok(());
    }
    let journal: Journal =
        serde_json::from_slice(&fs::read(directory.join("pending.json")).map_err(io_error)?)
            .map_err(io_error)?;
    let mut backup = fs::read(directory.join("original.exe")).map_err(io_error)?;
    validate_backup(&journal, executable, &mut backup)?;
    let mut file = exclusive(executable, false).map_err(io_error)?;
    let current = hash(&bytes_from(&mut file)?);
    if current == journal.original_sha256 {
        // Already restored, or this version originally allowed Inspector.
    } else if current == journal.patched_sha256 {
        write_byte(&mut file, journal.offset, journal.original_value)?;
        if hash(&bytes_from(&mut file)?) != journal.original_sha256 {
            return Err(io_error("恢复后哈希不一致，保留原始备份，请勿继续启动。"));
        }
    } else {
        return Err(io_error("Grok EXE 已更新或被其他程序修改；为防止覆盖新版，未执行还原。请检查 fuse-recovery 备份与记录。"));
    }
    clear_record(directory)
}

pub struct LaunchLease {
    lock: Option<File>,
    helper_lease: Option<HelperLease>,
    pub nonce: String,
}

struct HelperLease {
    child: Child,
    _stdin: ChildStdin,
}

impl LaunchLease {
    pub fn ensure_helper_alive(&mut self) -> Result<(), String> {
        if let Some(helper) = &mut self.helper_lease {
            if helper.child.try_wait().map_err(io_error)?.is_some() {
                // The transaction lock is still held, so a replacement cannot restore too early.
                self.helper_lease = Some(start_helper()?);
            }
        }
        Ok(())
    }
}

impl Drop for LaunchLease {
    fn drop(&mut self) {
        // Once the child has started (or launch failed), let the helper decide when it is safe.
        self.lock.take();
        self.helper_lease.take();
    }
}

fn start_helper() -> Result<HelperLease, String> {
    // The host kills all plugin-root executables on disable/update. Keep a
    // content-addressed helper outside that root, and outside a kill-on-close job.
    let source = fs::read(std::env::current_exe().map_err(io_error)?).map_err(io_error)?;
    let helper_path = root().join(format!("recovery-helper-{}.exe", hash(&source)));
    if helper_path.exists() {
        if hash(&fs::read(&helper_path).map_err(io_error)?) != hash(&source) {
            return Err(io_error("恢复助手文件校验失败。"));
        }
    } else {
        let temporary = root().join(format!("helper-{}.tmp", uuid::Uuid::new_v4()));
        write_new(&temporary, &source)?;
        if let Err(error) = publish(&temporary, &helper_path) {
            let _ = fs::remove_file(&temporary);
            if !fs::read(&helper_path)
                .map(|bytes| bytes == source)
                .unwrap_or(false)
            {
                return Err(error);
            }
        }
    }
    let mut child = Command::new(helper_path)
        .arg(HELPER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .map_err(io_error)?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| io_error("恢复助手没有输出管道"))?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(line)) if line.trim() == "FUSE_RECOVERY_READY_V1" => {
            let input = child
                .stdin
                .take()
                .ok_or_else(|| io_error("恢复助手没有租约管道"))?;
            Ok(HelperLease {
                child,
                _stdin: input,
            })
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Err(io_error("独立恢复助手未就绪，未修改程序。"))
        }
    }
}

pub fn prepare_launch(executable: &Path, inspector_port: u16) -> Result<LaunchLease, String> {
    if normalized(executable) != normalized(Path::new(TARGET)) {
        return Err(io_error("拒绝修改非预期 Grok 可执行文件。"));
    }
    let directory = root();
    let mut lease = LaunchLease {
        lock: Some(wait_for_lock(&directory)?),
        helper_lease: None,
        nonce: uuid::Uuid::new_v4().to_string(),
    };
    wait_for_exit()?;
    restore_locked(&directory, executable)?;
    let mut file = exclusive(executable, false).map_err(io_error)?;
    let mut bytes = bytes_from(&mut file)?;
    let offset = inspect_offset(&bytes)?;
    let original_value = bytes[offset];
    // Helper must acknowledge readiness BEFORE the first modification.
    lease.helper_lease = Some(start_helper()?);
    let original_sha256 = hash(&bytes);
    bytes[offset] = b'1';
    let patched_sha256 = hash(&bytes);
    bytes[offset] = original_value;
    let journal = Journal {
        schema: 1,
        executable: executable.to_string_lossy().into_owned(),
        offset,
        original_value,
        original_sha256,
        patched_sha256,
        inspector_port,
        nonce: lease.nonce.clone(),
    };
    prepare_record(&directory, &journal, &bytes)?;
    lease.ensure_helper_alive()?;
    if original_value == b'0' {
        write_byte(&mut file, offset, b'1')?;
    }
    if hash(&bytes_from(&mut file)?) != journal.patched_sha256 {
        return Err(io_error(
            "临时修改校验失败，恢复助手将尝试还原；未启动 Grok。",
        ));
    }
    Ok(lease)
}

pub fn restore_before_official_launch() -> Result<File, String> {
    let directory = root();
    // A helper may be hashing/restoring the same file. Wait for its transaction, not its lifetime.
    let guard = wait_for_lock(&directory)?;
    wait_for_exit()?;
    restore_locked(&directory, Path::new(TARGET))?;
    // Caller holds this through the ordinary CreateProcess, excluding another lease.
    Ok(guard)
}

pub fn resume_recovery() -> Result<(), String> {
    if root().join("pending.json").exists() {
        drop(start_helper()?);
    }
    Ok(())
}

pub fn recovery_error() -> Option<String> {
    read_recovery_error(&root())
}

#[derive(Serialize, Deserialize)]
struct RecoveryError {
    journal_sha256: String,
    message: String,
}

fn read_recovery_error(directory: &Path) -> Option<String> {
    let pending = fs::read(directory.join("pending.json")).ok()?;
    let record: RecoveryError =
        serde_json::from_slice(&fs::read(directory.join("recovery-error.txt")).ok()?).ok()?;
    (record.journal_sha256 == hash(&pending)).then_some(record.message)
}

// Caller holds transaction.lock. Bind errors to the complete immutable journal
// (including nonce), so late network results cannot poison another transaction.
fn record_recovery_result(directory: &Path, pending: &[u8], result: Result<(), String>) {
    if fs::read(directory.join("pending.json")).ok().as_deref() != Some(pending) {
        return;
    }
    match result {
        Ok(()) => {
            let _ = fs::remove_file(directory.join("recovery-error.txt"));
        }
        Err(message) => {
            let record = RecoveryError {
                journal_sha256: hash(pending),
                message,
            };
            if let Ok(bytes) = serde_json::to_vec(&record) {
                let _ = fs::write(directory.join("recovery-error.txt"), bytes);
            }
        }
    }
}

pub fn is_helper() -> bool {
    std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(HELPER_ARG))
}

pub fn run_helper() -> Result<(), String> {
    let directory = root();
    fs::create_dir_all(&directory).map_err(io_error)?;
    println!("FUSE_RECOVERY_READY_V1");
    std::io::stdout().flush().map_err(io_error)?;
    // EOF means either the launch transaction completed or the worker crashed.
    std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink()).map_err(io_error)?;
    // Each helper rechecks under the transaction lock. Do not exit merely because
    // an older helper exists: it may already be finishing a previous transaction.
    loop {
        // Always inspect the record under the same lock used by prepare_launch.
        if let Ok(_guard) = lock(&directory) {
            if !directory.join("pending.json").exists() {
                return Ok(());
            }
            let pending = fs::read(directory.join("pending.json")).map_err(io_error)?;
            match snapshot_executable_processes(TARGET) {
                Ok(processes) if processes.is_empty() => {
                    match restore_locked(&directory, Path::new(TARGET)) {
                        Ok(()) => return Ok(()),
                        Err(error) => {
                            // Keep evidence and retry: antivirus and late-exiting children can hold handles.
                            record_recovery_result(&directory, &pending, Err(error));
                        }
                    }
                }
                Err(error) => {
                    record_recovery_result(&directory, &pending, Err(error));
                }
                Ok(processes) => {
                    let journal = (|| {
                        let journal: Journal = serde_json::from_slice(
                            &fs::read(directory.join("pending.json")).map_err(io_error)?,
                        )
                        .map_err(io_error)?;
                        if journal.schema != 1
                            || normalized(Path::new(&journal.executable))
                                != normalized(Path::new(TARGET))
                            || uuid::Uuid::parse_str(&journal.nonce).is_err()
                        {
                            return Err(io_error("恢复助手拒绝无效启动记录。"));
                        }
                        Ok(journal)
                    })();
                    // No file writes follow. Never hold the disk transaction
                    // lock during a network operation against the Inspector.
                    drop(_guard);
                    let cleanup = journal.and_then(|journal| {
                        crate::electron_wco::close_interrupted_inspector(
                            journal.inspector_port,
                            &journal.nonce,
                            &processes,
                        )
                    });
                    if let Ok(_guard) = lock(&directory) {
                        record_recovery_result(&directory, &pending, cleanup);
                    }
                }
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
        original: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("grok-fuse-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let executable = root.join("Grok Bot.exe");
            let mut original = b"MZ synthetic test image ".to_vec();
            original.extend_from_slice(SENTINEL);
            original.extend_from_slice(b"\x01\x09010011011 trailing bytes");
            fs::write(&executable, &original).unwrap();
            Self {
                root,
                executable,
                original,
            }
        }
        fn prepare(&self) -> Journal {
            let offset = inspect_offset(&self.original).unwrap();
            let mut patched = self.original.clone();
            patched[offset] = b'1';
            let journal = Journal {
                schema: 1,
                executable: self.executable.to_string_lossy().into_owned(),
                offset,
                original_value: self.original[offset],
                original_sha256: hash(&self.original),
                patched_sha256: hash(&patched),
                inspector_port: 9238,
                nonce: uuid::Uuid::new_v4().to_string(),
            };
            prepare_record(&self.root, &journal, &self.original).unwrap();
            fs::write(&self.executable, patched).unwrap();
            journal
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn restores_exact_original_and_removes_journal() {
        let f = Fixture::new();
        f.prepare();
        restore_locked(&f.root, &f.executable).unwrap();
        assert_eq!(fs::read(&f.executable).unwrap(), f.original);
        assert!(!f.root.join("pending.json").exists());
        assert!(!f.root.join("original.exe").exists());
    }
    #[test]
    fn crash_before_patch_or_after_restore_is_idempotent() {
        let f = Fixture::new();
        f.prepare();
        fs::write(&f.executable, &f.original).unwrap();
        restore_locked(&f.root, &f.executable).unwrap();
        restore_locked(&f.root, &f.executable).unwrap();
        assert_eq!(fs::read(&f.executable).unwrap(), f.original);
    }
    #[test]
    fn never_overwrites_updated_executable() {
        let f = Fixture::new();
        f.prepare();
        fs::write(&f.executable, b"new official release").unwrap();
        assert!(restore_locked(&f.root, &f.executable)
            .unwrap_err()
            .contains("已更新"));
        assert_eq!(fs::read(&f.executable).unwrap(), b"new official release");
        assert!(f.root.join("pending.json").exists());
        assert!(f.root.join("original.exe").exists());
    }
    #[test]
    fn corrupt_backup_or_record_cannot_modify_executable() {
        let f = Fixture::new();
        f.prepare();
        let before = fs::read(&f.executable).unwrap();
        fs::write(f.root.join("original.exe"), b"corrupt").unwrap();
        assert!(restore_locked(&f.root, &f.executable).is_err());
        assert_eq!(fs::read(&f.executable).unwrap(), before);
    }
    #[test]
    fn rejects_wrong_target_and_multi_byte_patch_records() {
        let f = Fixture::new();
        let mut journal = f.prepare();
        let mut original = f.original.clone();
        assert!(validate_backup(&journal, Path::new("another.exe"), &mut original).is_err());
        journal.patched_sha256 = journal.original_sha256.clone();
        assert!(validate_backup(&journal, &f.executable, &mut original).is_err());
    }
    #[test]
    fn exclusive_lock_is_released_without_deleting_file() {
        let f = Fixture::new();
        let guard = lock(&f.root).unwrap();
        assert!(lock(&f.root).is_err());
        drop(guard);
        assert!(lock(&f.root).is_ok());
    }
    #[test]
    fn does_not_modify_locked_executable() {
        let f = Fixture::new();
        f.prepare();
        let file = exclusive(&f.executable, false).unwrap();
        assert!(restore_locked(&f.root, &f.executable).is_err());
        drop(file);
        restore_locked(&f.root, &f.executable).unwrap();
    }
    #[test]
    fn rejects_ambiguous_removed_truncated_or_unknown_fuses() {
        let f = Fixture::new();
        let offset = inspect_offset(&f.original).unwrap();
        let mut bytes = f.original.clone();
        bytes[offset] = b'r';
        assert!(inspect_offset(&bytes).is_err());
        bytes = f.original.clone();
        bytes.extend_from_slice(SENTINEL);
        assert!(inspect_offset(&bytes).is_err());
        assert!(inspect_offset(&f.original[..offset]).is_err());
        bytes = f.original.clone();
        let version = offset - INSPECT_INDEX - 2;
        bytes[version] = 2;
        assert!(inspect_offset(&bytes).is_err());
    }

    #[test]
    fn preserves_inspector_when_originally_enabled() {
        let mut f = Fixture::new();
        let offset = inspect_offset(&f.original).unwrap();
        f.original[offset] = b'1';
        fs::write(&f.executable, &f.original).unwrap();
        f.prepare();
        restore_locked(&f.root, &f.executable).unwrap();
        assert_eq!(fs::read(&f.executable).unwrap(), f.original);
    }

    #[test]
    fn archives_orphan_backup_without_using_it_for_new_version() {
        let f = Fixture::new();
        fs::write(f.root.join("original.exe"), b"previous release backup").unwrap();
        f.prepare();
        restore_locked(&f.root, &f.executable).unwrap();
        assert_eq!(fs::read(&f.executable).unwrap(), f.original);
        let orphan = fs::read_dir(&f.root)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name().to_string_lossy().starts_with("orphan-"))
            .unwrap();
        assert_eq!(fs::read(orphan.path()).unwrap(), b"previous release backup");
    }

    #[test]
    fn malformed_journal_never_touches_executable_or_backup() {
        let f = Fixture::new();
        f.prepare();
        let before = fs::read(&f.executable).unwrap();
        fs::write(f.root.join("pending.json"), b"{unfinished").unwrap();
        assert!(restore_locked(&f.root, &f.executable).is_err());
        assert_eq!(fs::read(&f.executable).unwrap(), before);
        assert_eq!(fs::read(f.root.join("original.exe")).unwrap(), f.original);
    }

    #[test]
    fn recovery_errors_expire_on_success_or_transaction_change() {
        let f = Fixture::new();
        f.prepare();
        let pending = fs::read(f.root.join("pending.json")).unwrap();
        record_recovery_result(&f.root, &pending, Err("temporary".into()));
        assert_eq!(read_recovery_error(&f.root).as_deref(), Some("temporary"));
        record_recovery_result(&f.root, &pending, Ok(()));
        assert!(read_recovery_error(&f.root).is_none());
        record_recovery_result(&f.root, &pending, Err("old".into()));
        fs::write(f.root.join("pending.json"), b"another transaction").unwrap();
        record_recovery_result(&f.root, &pending, Err("late result".into()));
        assert!(read_recovery_error(&f.root).is_none());
        fs::remove_file(f.root.join("pending.json")).unwrap();
        record_recovery_result(&f.root, &pending, Err("late result".into()));
        assert!(read_recovery_error(&f.root).is_none());
    }
}
