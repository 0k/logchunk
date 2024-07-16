use crate::apply;

pub fn run(args: &Vec<String>) -> Result<bool, String> {
    match apply::wrap(args) {
        Ok(_) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}
