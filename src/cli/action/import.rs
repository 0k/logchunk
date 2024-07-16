use crate::import;

pub fn run(label: &str, sqlite_db_path: &str, failed_chunk_path: &str) -> Result<bool, String> {
    match import::load(label, sqlite_db_path, failed_chunk_path) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e == "No content".to_string() {
                return Ok(false);
            }
            Err(e.to_string())
        }
    }
}
