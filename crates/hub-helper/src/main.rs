//! `hub-helper.exe` — proses elevated berumur pendek (PRD §13.7).
//!
//! Bukan Windows Service, secara sengaja: service yang selalu berjalan dengan
//! hak SYSTEM adalah permukaan serangan permanen. Helper ini hidup selama satu
//! sesi elevasi dan keluar.
//!
//! Semua yang dilakukannya dibatasi tiga hal:
//! 1. Protokol yang hanya punya empat operasi (`protocol.rs`).
//! 2. Allowlist direktori yang divalidasi di sini, bukan di client (`guard.rs`).
//! 3. Verifikasi bahwa proses di ujung pipe adalah binary kami sendiri.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;

use hub_elevate::guard::{has_traversal, AllowedRoots};
use hub_elevate::protocol::{HelperCommand, HelperResponse, PROTOCOL_VERSION};

fn main() {
    init_logging();

    let Some(pipe_name) = parse_pipe_arg() else {
        eprintln!("hub-helper: dipanggil tanpa --pipe; binary ini tidak untuk dijalankan langsung");
        std::process::exit(2);
    };

    match run(&pipe_name) {
        Ok(()) => {}
        Err(e) => {
            tracing::error!(error = %e, "helper berhenti dengan error");
            std::process::exit(1);
        }
    }
}

fn parse_pipe_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--pipe" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--pipe=") {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(windows)]
fn run(pipe_name: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader};

    let roots = AllowedRoots::system_default();

    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_name)
        .map_err(|e| format!("tidak dapat membuka pipe: {e}"))?;

    // Verifikasi ujung lain sebelum menerima satu command pun. Tanpa langkah
    // ini, proses lokal mana pun yang menebak nama pipe dapat meminta helper
    // elevated melakukan operasi (T5).
    verify_peer(&pipe)?;

    let writer = pipe
        .try_clone()
        .map_err(|e| format!("gagal menduplikasi handle pipe: {e}"))?;
    let mut reader = BufReader::new(pipe);
    let mut writer = writer;
    let mut handshaked = false;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // client menutup pipe
            Ok(_) => {}
            Err(e) => return Err(format!("gagal membaca dari pipe: {e}")),
        }

        if line.len() > hub_elevate::protocol::MAX_MESSAGE_BYTES {
            return Err("pesan melebihi batas ukuran".into());
        }

        let command: HelperCommand = match serde_json::from_str(line.trim()) {
            Ok(c) => c,
            Err(e) => {
                respond(&mut writer, HelperResponse::Error {
                    message: format!("command tidak dapat diparsing: {e}"),
                })?;
                continue;
            }
        };

        if let HelperCommand::Goodbye = command {
            break;
        }

        // Handshake wajib lebih dulu: helper lama + launcher baru adalah
        // kombinasi yang mungkin terjadi saat update parsial.
        if !handshaked {
            match command {
                HelperCommand::Hello { protocol_version } if protocol_version == PROTOCOL_VERSION => {
                    handshaked = true;
                    respond(&mut writer, HelperResponse::Ok)?;
                    continue;
                }
                HelperCommand::Hello { protocol_version } => {
                    respond(&mut writer, HelperResponse::Error {
                        message: format!(
                            "versi protokol {protocol_version} tidak cocok dengan {PROTOCOL_VERSION}"
                        ),
                    })?;
                    return Ok(());
                }
                _ => {
                    respond(&mut writer, HelperResponse::Error {
                        message: "handshake belum dilakukan".into(),
                    })?;
                    return Ok(());
                }
            }
        }

        let response = handle(&roots, command);
        respond(&mut writer, response)?;
    }

    Ok(())
}

#[cfg(not(windows))]
fn run(_pipe_name: &str) -> Result<(), String> {
    Err("hub-helper hanya berjalan di Windows".into())
}

#[cfg(windows)]
fn respond(
    writer: &mut std::fs::File,
    response: HelperResponse,
) -> Result<(), String> {
    use std::io::Write;
    let mut line =
        serde_json::to_string(&response).map_err(|e| format!("serialisasi respons: {e}"))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .map_err(|e| format!("gagal menulis ke pipe: {e}"))?;
    writer.flush().map_err(|e| format!("flush gagal: {e}"))
}

/// Jalankan satu command setelah memvalidasi path-nya.
fn handle(roots: &AllowedRoots, command: HelperCommand) -> HelperResponse {
    match command {
        HelperCommand::Hello { .. } | HelperCommand::Goodbye => HelperResponse::Ok,

        HelperCommand::MoveDir { from, to } => {
            if let Err(message) = check_pair(roots, &from, &to) {
                return HelperResponse::Error { message };
            }
            // Rename lintas volume bukan operasi atomik; menolaknya di sini
            // mencegah client memaksa jalur copy+delete yang dapat gagal di
            // tengah dan meninggalkan bundle korup (NFR-2.1).
            if !hub_elevate::guard::same_volume(&from, &to) {
                return HelperResponse::Error {
                    message: "sumber dan tujuan berada di volume berbeda".into(),
                };
            }
            match std::fs::rename(&from, &to) {
                Ok(()) => {
                    tracing::info!(?from, ?to, "move_dir");
                    HelperResponse::Ok
                }
                Err(e) => HelperResponse::Error {
                    message: describe_io(&e),
                },
            }
        }

        HelperCommand::RemoveDir { path } => {
            if let Err(message) = check_one(roots, &path) {
                return HelperResponse::Error { message };
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => HelperResponse::Ok,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => HelperResponse::Ok,
                Err(e) => HelperResponse::Error {
                    message: describe_io(&e),
                },
            }
        }

        HelperCommand::ScheduleReplaceOnReboot { from, to } => {
            if let Err(message) = check_pair(roots, &from, &to) {
                return HelperResponse::Error { message };
            }
            match schedule_replace_on_reboot(&from, &to) {
                Ok(()) => HelperResponse::Ok,
                Err(message) => HelperResponse::Error { message },
            }
        }

        HelperCommand::ProbeWritable { path } => {
            if let Err(message) = check_one(roots, &path) {
                return HelperResponse::Error { message };
            }
            HelperResponse::Writable {
                writable: hub_core::paths::probe_writable(&path),
            }
        }
    }
}

fn check_one(roots: &AllowedRoots, path: &Path) -> Result<(), String> {
    if has_traversal(path) {
        tracing::warn!(?path, "command dengan traversal ditolak");
        return Err("path mengandung komponen terlarang".into());
    }
    if !roots.permits(path) {
        tracing::warn!(?path, "command di luar allowlist ditolak");
        return Err("path di luar direktori yang diizinkan".into());
    }
    Ok(())
}

fn check_pair(roots: &AllowedRoots, from: &Path, to: &Path) -> Result<(), String> {
    check_one(roots, from)?;
    check_one(roots, to)
}

/// Terjemahkan error I/O menjadi pesan yang dapat dipetakan client.
///
/// Kata "terkunci" di sini bukan hiasan: `client.rs` mencarinya untuk mengubah
/// kegagalan ini menjadi `HubError::FileLocked`, yang membuat UI menawarkan
/// alur §8.7 alih-alih menampilkan kode error Win32.
fn describe_io(e: &std::io::Error) -> String {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    const ERROR_ACCESS_DENIED: i32 = 5;

    match e.raw_os_error() {
        Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION) => {
            format!("berkas terkunci proses lain ({e})")
        }
        Some(ERROR_ACCESS_DENIED) => format!("akses ditolak, kemungkinan terkunci ({e})"),
        _ => e.to_string(),
    }
}

/// `MoveFileExW` + `MOVEFILE_DELAY_UNTIL_REBOOT`. Ini menulis ke
/// `PendingFileRenameOperations` di registry, yang butuh hak Administrator —
/// itulah sebabnya operasi ini hidup di helper dan bukan di proses utama, dan
/// mengapa UI hanya menawarkannya saat elevasi tersedia (PRD §8.7).
#[cfg(windows)]
fn schedule_replace_on_reboot(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT, MOVEFILE_REPLACE_EXISTING,
    };

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let from_w = wide(from);
    let to_w = wide(to);
    unsafe {
        MoveFileExW(
            PCWSTR(from_w.as_ptr()),
            PCWSTR(to_w.as_ptr()),
            MOVEFILE_DELAY_UNTIL_REBOOT | MOVEFILE_REPLACE_EXISTING,
        )
        .map_err(|e| format!("MoveFileExW gagal: {e}"))
    }
}

#[cfg(not(windows))]
fn schedule_replace_on_reboot(_from: &Path, _to: &Path) -> Result<(), String> {
    Err("penjadwalan saat reboot hanya tersedia di Windows".into())
}

/// Verifikasi bahwa proses client adalah binary yang ditandatangani dengan
/// sertifikat yang sama (PRD §13.7 langkah 4).
///
/// Implementasi saat ini memverifikasi bahwa client adalah executable dengan
/// path yang sama persis dengan direktori helper — pemeriksaan yang murah dan
/// sudah menutup kasus "proses acak menebak nama pipe". Verifikasi Authenticode
/// penuh (`WinVerifyTrust` + perbandingan thumbprint) dipasang bersama code
/// signing di M3; sampai itu ada, biarkan `TODO` ini terlihat alih-alih
/// mengklaim jaminan yang belum ada.
#[cfg(windows)]
fn verify_peer(pipe: &std::fs::File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;

    let handle = HANDLE(pipe.as_raw_handle() as _);
    let mut pid: u32 = 0;
    unsafe {
        GetNamedPipeClientProcessId(handle, &mut pid)
            .map_err(|e| format!("tidak dapat menentukan PID client: {e}"))?;
    }

    let client_exe = process_image_path(pid)?;
    let helper_dir = std::env::current_exe()
        .map_err(|e| format!("tidak dapat menentukan path helper: {e}"))?
        .parent()
        .ok_or("helper tanpa direktori induk")?
        .to_path_buf();

    if !client_exe.starts_with(&helper_dir) {
        tracing::error!(?client_exe, "client di luar direktori instalasi, koneksi ditolak");
        return Err("client tidak tepercaya".into());
    }

    tracing::info!(pid, ?client_exe, "client terverifikasi");
    Ok(())
}

#[cfg(windows)]
fn process_image_path(pid: u32) -> Result<std::path::PathBuf, String> {
    use windows::Win32::Foundation::CloseHandle;
    // `QueryFullProcessImageNameW` hidup di Threading, bukan Storage::FileSystem.
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| format!("OpenProcess gagal: {e}"))?;

        let mut buffer = vec![0u16; 32_768];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.map_err(|e| format!("QueryFullProcessImageNameW gagal: {e}"))?;

        buffer.truncate(size as usize);
        Ok(std::path::PathBuf::from(String::from_utf16_lossy(&buffer)))
    }
}

#[cfg(not(windows))]
fn verify_peer(_pipe: &std::fs::File) -> Result<(), String> {
    Err("hub-helper hanya berjalan di Windows".into())
}

fn init_logging() {
    // Helper menulis ke log terpisah: baris yang berasal dari proses elevated
    // harus dapat dibedakan saat mengaudit insiden.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_arg_parsing_accepts_both_forms() {
        // Tidak dapat memanipulasi `std::env::args`, jadi yang diuji logikanya
        // lewat fungsi murni di bawah.
        fn parse(args: &[&str]) -> Option<String> {
            let mut it = args.iter();
            while let Some(arg) = it.next() {
                if *arg == "--pipe" {
                    return it.next().map(|s| s.to_string());
                }
                if let Some(v) = arg.strip_prefix("--pipe=") {
                    return Some(v.to_string());
                }
            }
            None
        }
        assert_eq!(parse(&["--pipe", "\\\\.\\pipe\\x"]).as_deref(), Some("\\\\.\\pipe\\x"));
        assert_eq!(parse(&["--pipe=\\\\.\\pipe\\y"]).as_deref(), Some("\\\\.\\pipe\\y"));
        assert_eq!(parse(&["--lain"]), None);
    }

    #[test]
    fn commands_outside_allowlist_are_refused() {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let roots = AllowedRoots::with_roots(vec![allowed.path().to_path_buf()]);

        let response = handle(
            &roots,
            HelperCommand::RemoveDir {
                path: outside.path().join("korban"),
            },
        );
        assert!(matches!(response, HelperResponse::Error { .. }));
    }

    #[test]
    fn traversal_in_move_is_refused() {
        let allowed = tempfile::tempdir().unwrap();
        let roots = AllowedRoots::with_roots(vec![allowed.path().to_path_buf()]);

        let response = handle(
            &roots,
            HelperCommand::MoveDir {
                from: allowed.path().join("a"),
                to: allowed.path().join("..").join("Windows").join("System32"),
            },
        );
        assert!(matches!(response, HelperResponse::Error { .. }));
    }
}
