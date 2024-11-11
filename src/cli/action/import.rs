use crate::import;

pub fn run(label: &str, sqlite_db_path: &str, failed_chunks_folder: &str) -> Result<bool, String> {
    match import::load(label, sqlite_db_path, failed_chunks_folder) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e == "No content".to_string() {
                return Ok(false);
            }
            Err(e.to_string())
        }
    }
}
