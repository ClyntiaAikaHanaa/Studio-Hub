//! Sisi client: spawn helper elevated dan bicara lewat named pipe (PRD §13.7).
//!
//! Alur:
//! 1. Buat named pipe dengan nama acak.
//! 2. `ShellExecuteW` dengan verb `runas` → UAC prompt.
//! 3. Helper terhubung, memverifikasi signature proses client.
//! 4. Kirim command, terima hasil.
//! 5. `Goodbye` → helper keluar.
//!
//! Beberapa operasi dibundel dalam **satu** sesi helper, sehingga "Update all"
//! ke lokasi sistem berarti satu UAC prompt, bukan lima (PRD §13.7).

use std::path::{Path, PathBuf};

use hub_core::install::Elevator;
use hub_core::{HubError, Result};

use crate::protocol::{HelperCommand, HelperResponse, PROTOCOL_VERSION};

pub struct ElevatedSession {
    #[cfg(windows)]
    pipe: std::sync::Mutex<transport::PipeServer>,
    helper_path: PathBuf,
}

impl ElevatedSession {
    /// Mulai sesi elevated. Ini memicu UAC prompt — panggil hanya tepat sebelum
    /// operasi tulis yang membutuhkannya, tidak saat startup (ADR-2, NFR-3.3).
    pub fn start(helper_path: &Path) -> Result<Self> {
        if !helper_path.is_file() {
            return Err(HubError::internal(format!(
                "hub-helper.exe tidak ditemukan di {}",
                helper_path.display()
            )));
        }

        #[cfg(windows)]
        {
            let session_id = uuid::Uuid::new_v4();
            let pipe_name = crate::protocol::session_pipe_name(&session_id);

            let mut server = transport::PipeServer::create(&pipe_name)?;
            transport::spawn_elevated(helper_path, &pipe_name)?;
            server.wait_for_client()?;

            let session = ElevatedSession {
                pipe: std::sync::Mutex::new(server),
                helper_path: helper_path.to_path_buf(),
            };
            match session.send(HelperCommand::Hello {
                protocol_version: PROTOCOL_VERSION,
            })? {
                HelperResponse::Ok => Ok(session),
                HelperResponse::Error { message } => Err(HubError::internal(format!(
                    "helper menolak handshake: {message}"
                ))),
                _ => Err(HubError::internal("respons handshake tidak terduga")),
            }
        }

        #[cfg(not(windows))]
        {
            let _ = helper_path;
            Err(HubError::ElevationDenied)
        }
    }

    pub fn helper_path(&self) -> &Path {
        &self.helper_path
    }

    #[cfg(windows)]
    fn send(&self, command: HelperCommand) -> Result<HelperResponse> {
        let mut pipe = self
            .pipe
            .lock()
            .map_err(|_| HubError::internal("mutex pipe teracuni"))?;
        pipe.send(&command)
    }

    #[cfg(not(windows))]
    fn send(&self, _command: HelperCommand) -> Result<HelperResponse> {
        Err(HubError::ElevationDenied)
    }

    fn expect_ok(&self, command: HelperCommand) -> Result<()> {
        match self.send(command)? {
            HelperResponse::Ok => Ok(()),
            HelperResponse::Error { message } => Err(map_helper_error(&message)),
            HelperResponse::Writable { .. } => {
                Err(HubError::internal("respons helper tidak terduga"))
            }
        }
    }

    pub fn probe_writable(&self, path: &Path) -> Result<bool> {
        match self.send(HelperCommand::ProbeWritable {
            path: path.to_path_buf(),
        })? {
            HelperResponse::Writable { writable } => Ok(writable),
            HelperResponse::Error { message } => Err(map_helper_error(&message)),
            HelperResponse::Ok => Err(HubError::internal("respons helper tidak terduga")),
        }
    }
}

impl Drop for ElevatedSession {
    fn drop(&mut self) {
        // Helper keluar sendiri saat pipe ditutup, tapi `Goodbye` membuat
        // keluarnya bersih dan log-nya jelas.
        let _ = self.send(HelperCommand::Goodbye);
    }
}

impl Elevator for ElevatedSession {
    fn move_dir(&self, from: &Path, to: &Path) -> Result<()> {
        self.expect_ok(HelperCommand::MoveDir {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        })
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        self.expect_ok(HelperCommand::RemoveDir {
            path: path.to_path_buf(),
        })
    }

    fn schedule_replace_on_reboot(&self, from: &Path, to: &Path) -> Result<()> {
        self.expect_ok(HelperCommand::ScheduleReplaceOnReboot {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        })
    }
}

fn map_helper_error(message: &str) -> HubError {
    if message.contains("terkunci") || message.contains("sharing") {
        HubError::FileLocked {
            path: String::new(),
            holders: Vec::new(),
        }
    } else {
        HubError::internal(format!("helper: {message}"))
    }
}

/// Lokasi `hub-helper.exe`: selalu di samping executable utama.
///
/// Path absolut, tidak pernah dicari lewat `PATH` — mencari lewat `PATH` adalah
/// cara klasik mengeksekusi binary yang salah (mitigasi T6).
pub fn default_helper_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| HubError::internal("executable tanpa direktori induk"))?;
    Ok(dir.join("hub-helper.exe"))
}

#[cfg(windows)]
mod transport {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use hub_core::{HubError, Result};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, WRITE_DAC};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use crate::protocol::{HelperCommand, HelperResponse, MAX_MESSAGE_BYTES};

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub struct PipeServer {
        reader: BufReader<std::fs::File>,
        writer: std::fs::File,
    }

    impl PipeServer {
        pub fn create(name: &str) -> Result<Self> {
            let wide_name = wide(name);
            // DACL default pipe mewarisi token pembuat, yang berarti hanya
            // pengguna saat ini (dan SYSTEM) yang dapat membukanya. Itu
            // properti yang kita andalkan di T5 — jangan longgarkan.
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(wide_name.as_ptr()),
                    PIPE_ACCESS_DUPLEX
                        | windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(
                            WRITE_DAC.0,
                        ),
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    1, // satu instance: tidak ada ruang untuk client kedua
                    MAX_MESSAGE_BYTES as u32,
                    MAX_MESSAGE_BYTES as u32,
                    30_000,
                    None,
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                return Err(HubError::internal("gagal membuat named pipe"));
            }

            let owned = unsafe { OwnedHandle::from_raw_handle(handle.0 as _) };
            let file = std::fs::File::from(owned);
            let writer = file.try_clone()?;
            Ok(PipeServer {
                reader: BufReader::new(file),
                writer,
            })
        }

        pub fn wait_for_client(&mut self) -> Result<()> {
            // `ConnectNamedPipe` memakai handle yang sama; kita meminjamnya
            // kembali dari file.
            use std::os::windows::io::AsRawHandle;
            let handle = HANDLE(self.writer.as_raw_handle() as _);
            unsafe {
                // ERROR_PIPE_CONNECTED berarti client sudah terhubung duluan —
                // itu sukses, bukan kegagalan.
                if let Err(e) = ConnectNamedPipe(handle, None) {
                    const ERROR_PIPE_CONNECTED: i32 = 535;
                    if e.code().0 & 0xffff != ERROR_PIPE_CONNECTED {
                        return Err(HubError::internal(format!(
                            "client helper tidak terhubung: {e}"
                        )));
                    }
                }
            }
            Ok(())
        }

        pub fn send(&mut self, command: &HelperCommand) -> Result<HelperResponse> {
            let mut line = serde_json::to_string(command)
                .map_err(|e| HubError::internal(format!("serialisasi command: {e}")))?;
            line.push('\n');
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;

            let mut response = String::new();
            let read = self.reader.read_line(&mut response)?;
            if read == 0 {
                return Err(HubError::internal("helper menutup pipe"));
            }
            serde_json::from_str(response.trim()).map_err(|e| {
                HubError::internal(format!("respons helper tidak dapat diparsing: {e}"))
            })
        }
    }

    /// Jalankan helper dengan verb `runas`. Ini yang memunculkan UAC prompt.
    pub fn spawn_elevated(helper: &std::path::Path, pipe_name: &str) -> Result<()> {
        let file = wide(&helper.to_string_lossy());
        let verb = wide("runas");
        let params = wide(&format!("--pipe {pipe_name}"));

        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };

        unsafe {
            if let Err(e) = ShellExecuteExW(&mut info) {
                const ERROR_CANCELLED: i32 = 1223;
                // Pengguna menekan "No" di UAC. Ini bukan kegagalan sistem;
                // UI akan menawarkan instalasi per-user (§8.8).
                if e.code().0 & 0xffff == ERROR_CANCELLED {
                    return Err(HubError::ElevationDenied);
                }
                return Err(HubError::internal(format!("gagal menjalankan helper: {e}")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_path_is_absolute_and_beside_the_exe() {
        let path = default_helper_path().unwrap();
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "hub-helper.exe");
    }

    #[test]
    fn missing_helper_is_reported_before_any_uac_prompt() {
        // Penting bahwa ini gagal SEBELUM `ShellExecuteW`: helper yang hilang
        // tidak boleh memunculkan UAC prompt yang lalu tidak menghasilkan apa
        // pun. `ElevatedSession` tidak `Debug` (ia memegang handle pipe), jadi
        // hasilnya dicocokkan lewat `is_err` + pemeriksaan varian.
        let result = ElevatedSession::start(Path::new("Z:\\tidak\\ada\\hub-helper.exe"));
        assert!(result.is_err());
        match result {
            Err(HubError::Internal { .. }) => {}
            Err(other) => panic!("varian salah: {other:?}"),
            Ok(_) => unreachable!("sudah dipastikan Err di atas"),
        }
    }
}
