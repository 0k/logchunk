use crate::next;
use sha1::Digest;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;

pub fn run(file_name: &str, cursor_name: &str) -> Result<bool, String> {
    let log_file_path = crate::utils::normalize_path(&PathBuf::from(file_name))
        .map_err(|e| format!("Couldn't normalize path for '{}': {}.", file_name, e))?;

    // check for existence
    if !log_file_path.exists() {
        return Err(format!("No such file as '{}'.", log_file_path.display()));
    }

    let chunk_state_dir = std::env::var("CHUNK_STATE_DIR").unwrap_or_else(|_| {
        let exname = env::current_exe()
            .expect("Failed to get current executable path")
            .file_name()
            .expect("Failed to get executable name")
            .to_str()
            .expect("Failed to convert executable name to string")
            .to_string();
        format!("/var/spool/{}", exname)
    });
    // get sha1 of chunk_state_dir string
    let mut hasher = sha1::Sha1::new();
    hasher.update(log_file_path.to_str().unwrap());
    let result = hasher.finalize();
    let cursor_sha1 = format!("{:x}", result);
    let cursor_state_dir = format!("{}/{}/{}", chunk_state_dir, cursor_sha1, cursor_name);

    next::next_chunk(
        log_file_path,
        PathBuf::from_str(&cursor_state_dir).map_err(|e| e.to_string())?,
    )
}
