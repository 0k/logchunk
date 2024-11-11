use chrono::{format::ParseError, NaiveDateTime};
use regex::Regex;
use rusqlite::{params, Connection, Result as SqliteResult};
use std::io::{self, BufRead, Write};
use std::fs::File;
use std::cell::RefCell;
use std::rc::Rc;
use std::fmt::Write as OtherWrite;
use tempfile::NamedTempFile;


const ERR_CHUNK_INCOMPLETE: u8                   = 0b0000_0001;
const ERR_CHUNK_NO_START: u8                     = 0b0000_0010;
const ERR_CONNECTION_CLOSED: u8                  = 0b0000_0100;
const ERR_CONNECTION_CLOSED_WITOUT_FOLLOW_UP: u8 = 0b0000_1000;
const ERR_SIGNAL_RECEIVED: u8                    = 0b0001_0000;


pub trait BufReadExt: BufRead {
    fn split_with_delimiter(self, delim: u8) -> SplitWithDelimiter<Self>
    where
        Self: Sized,
    {
        SplitWithDelimiter {
            reader: self,
            delim,
        }
    }
}

impl<T: BufRead> BufReadExt for T {}

pub struct SplitWithDelimiter<R> {
    reader: R,
    delim: u8,
}

impl<R: BufRead> Iterator for SplitWithDelimiter<R> {
    type Item = std::io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<std::io::Result<Vec<u8>>> {
        let mut buf = Vec::new();
        match self.reader.read_until(self.delim, &mut buf) {
            Ok(0) => None, // EOF reached
            Ok(_) => Some(Ok(buf)),
            Err(e) => Some(Err(e)),
        }
    }
}


struct RsyncLog {
    label: String,
    start_time: i64,
    end_time: i64,
    total_bytes_sync: i64,
    total_bytes_sent: i64,
    total_bytes_sync_final: i64,
    total_bytes_sent_final: i64,
    num_files_changed: i64,
    chunk_parse_error: u8,
}

fn create_table_if_not_exists(conn: &Connection) -> SqliteResult<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rsync_log (
             id INTEGER PRIMARY KEY,
             label TEXT NOT NULL,
             start_time INTEGER NOT NULL,
             end_time INTEGER NOT NULL,
             total_bytes_sync INTEGER NOT NULL,
             total_bytes_sent INTEGER NOT NULL,
             total_bytes_sync_final INTEGER NOT NULL,
             total_bytes_sent_final INTEGER NOT NULL,
             num_files_changed INTEGER NOT NULL,
             chunk_parse_error INTEGER NOT NULL
         )",
        [],
    )?;
    Ok(())
}

fn insert_log(conn: &Connection, log: &RsyncLog) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO rsync_log (
             label,
             start_time,
             end_time,
             total_bytes_sync,
             total_bytes_sent,
             total_bytes_sync_final,
             total_bytes_sent_final,
             num_files_changed,
             chunk_parse_error
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            log.label,
            log.start_time.to_string(),
            log.end_time.to_string(),
            log.total_bytes_sync,
            log.total_bytes_sent,
            log.total_bytes_sync_final,
            log.total_bytes_sent_final,
            log.num_files_changed,
            log.chunk_parse_error,
        ],
    )?;
    Ok(())
}

fn parse_timestamp(s: &str) -> Result<NaiveDateTime, ParseError> {
    NaiveDateTime::parse_from_str(s, "%Y/%m/%d %H:%M:%S")
}


fn hex_dump(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        write!(&mut s, "{:02x}", byte).unwrap();
    }
    s
}

fn _mk_copy_lines_iter<'a>(
    iter: impl Iterator<Item = Result<Vec<u8>, std::io::Error>> + 'a,
    mut writer: File,
    last_line: Rc<RefCell<Vec<u8>>>,
    last_idx: Rc<RefCell<usize>>,
) -> impl Iterator<Item = Result<String, String>> + 'a {

    iter.enumerate().map(move |(idx, line)| {
        let mut line = line.map_err(|e| e.to_string())?;
        writer.write_all(&line).map_err(|e| e.to_string())?;

        if line.ends_with(b"\n") {
            line.pop();
        }

        // update the last line written
        *last_line.borrow_mut() = line.clone();
        *last_idx.borrow_mut() = idx;


        match String::from_utf8(line) {
            Ok(s) => {
                // log::trace!("Line {}: {:?}", idx, s);
                Ok(s)
            }
            Err(e) => {
                Err(format!("Invalid UTF-8 data [{}]", hex_dump(&e.into_bytes())))
            }
        }
    })
}

fn file_sha1(file: File) -> Result<String, String> {
    use sha1::{Digest, Sha1};
    use std::io::Read;

    let mut hasher = Sha1::new();
    let mut reader = std::io::BufReader::new(file);
    let mut buffer = [0; 4096];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let result = hasher.finalize();
    let mut s = String::new();
    for byte in result.iter() {
        write!(&mut s, "{:02x}", byte).unwrap();
    }
    Ok(s)
}


pub fn load(label: &str, sqlite_db_path: &str, failed_chunk_folder: &str) -> Result<(), String> {
    let stdin = io::stdin();
    let reader = stdin.lock().split_with_delimiter(b'\n');
    let failed_chunk_file = NamedTempFile::new()
        .map_err(|e| e.to_string())?;

    let last_line = Rc::new(RefCell::new(Vec::new()));
    let last_idx = Rc::new(RefCell::new(0));

    // make iterator that saves lines in failed_chunk_path
    let mut copy_lines_iter = _mk_copy_lines_iter(
        reader,
        failed_chunk_file.reopen().map_err(|e| e.to_string())?,
        last_line.clone(),
        last_idx.clone());

    // call load_iter with the iterator and catch the error
    let result = load_iter(label, &mut copy_lines_iter);
    match result {
        Ok(log) => {

            let conn = Connection::open(sqlite_db_path).map_err(|e| e.to_string())?;

            create_table_if_not_exists(&conn).map_err(|e| e.to_string())?;
            insert_log(&conn, &log).map_err(|e| e.to_string())?;

            log::info!("Log inserted into the database successfully.");
        }
        Err(e) => {
            if e == "No content".to_string() {
                return Err(e); // will automatically remove the chunk file
            }
            let sha1 = file_sha1(
                failed_chunk_file.reopen().map_err(|e| e.to_string())?
            ).map_err(|e| e.to_string())?;
            let failed_chunk_path = format!("{}/{}.chunk", failed_chunk_folder, sha1);
            // fetch last line of the failed chunk
            let last_failed_line = String::from_utf8_lossy(&last_line.borrow()).to_string();

            let last_idx = *last_idx.borrow() + 1;
            copy_lines_iter.for_each(|_| {});  // consume the iterator to save the failed chunk

            failed_chunk_file.persist(&failed_chunk_path).map_err(|e| e.to_string())?;
            return Err(format!("(Line {}) {}:\n  {}\n\n  Failed chunk is saved to {:?}", last_idx, e, last_failed_line, failed_chunk_path));
        }
    }

    Ok(())
}

fn load_iter(label: &str, reader: &mut impl Iterator<Item = Result<String, String>>) -> Result<RsyncLog, String> {

    let start_time: NaiveDateTime;
    let mut end_time: Option<NaiveDateTime> = None;
    let mut total_bytes_sync = 0;
    let mut total_bytes_sent = 0;
    let mut total_bytes_sync_final = 0;
    let mut total_bytes_sent_final = 0;
    let mut num_files_changed = 0;
    let mut chunk_parse_error = ERR_CHUNK_NO_START;
    let current_pid: String;

    let start_re =
        Regex::new(r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] receiving file list$")
            .map_err(|e| e.to_string())?;
    let end_re =
        Regex::new(r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] sent (\d+) bytes\s+received (\d+) bytes\s+total size (\d+)$")
        .map_err(|e| e.to_string())?;
    let file_change_re = Regex::new(
        r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] ([>.ch][L.dfsctp+]+ recv|\*deleting   del.) .* (\d+) (\d+)$",
    ).map_err(|e| e.to_string())?;
    let connection_closed_re = Regex::new(
        r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] rsync: connection unexpectedly closed \((\d+) bytes received so far\) \[generator\]$",
    )
    .map_err(|e| e.to_string())?;
    let connection_closed_followup_re = Regex::new(
        r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] rsync error: (.*)$",
    )
        .map_err(|e| e.to_string())?;
    let signal_received_re = Regex::new(
        r"^(\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}) \[(\d+)\] rsync error: received SIGINT, SIGTERM, or SIGHUP.*$",
    )
        .map_err(|e| e.to_string())?;


    if let Some(first_line) = reader.next() {
        let first_line = first_line.map_err(|e| e.to_string())?;
        if let Some(caps) = start_re.captures(&first_line) {
            start_time = parse_timestamp(&caps[1]).map_err(
                |e| format!("Failed to parse {:?} as a date ({}) on first chunk line",
                            &caps[1], e.to_string())
            )?;
            current_pid = caps[2].to_string();
            chunk_parse_error &= !ERR_CHUNK_NO_START;
            chunk_parse_error |= ERR_CHUNK_INCOMPLETE;
        } else {
            return Err(format!("Unexpected start line format"))
        }
    } else {
        return Err("No content".to_string());
    }

    for line in reader {
        let line = line.map_err(|e| e.to_string())?;
        if chunk_parse_error & ERR_CHUNK_INCOMPLETE == 0 {
            return Err("Unexpected lines found after the end line".to_string());
        }

        if chunk_parse_error & ERR_CONNECTION_CLOSED_WITOUT_FOLLOW_UP != 0 {
            if let Some(_caps) = connection_closed_followup_re.captures(&line) {
                chunk_parse_error &= !ERR_CONNECTION_CLOSED_WITOUT_FOLLOW_UP;
                chunk_parse_error &= !ERR_CHUNK_INCOMPLETE;
                continue
            } else {
                return Err("Expected follow-up line after connection closed".to_string());
            }
        }
        if let Some(caps) = end_re.captures(&line) {
            if current_pid != caps[2] {
                return Err(
                    format!("Unexpected PID change from {} to {} on last chunk line",
                            current_pid, &caps[2])
                );
            }
            end_time = Some(parse_timestamp(&caps[1]).map_err(
                |e| format!("Failed to parse {:?} as a date ({}) on last chunk line",
                            &caps[1], e.to_string())
            )?);
            total_bytes_sync_final = caps[5].parse::<i64>().map_err(
                |e| format!("Failed to parse total count {:?} as an i64 integer ({}) on last chunk line",
                            &caps[5], e.to_string())
            )?;
            total_bytes_sent_final = caps[4].parse::<i64>().map_err(
                |e| format!("Failed to parse received count {:?} as an i64 integer ({}) on last chunk line",
                            &caps[4], e.to_string())
            )?;
            chunk_parse_error &= !ERR_CHUNK_INCOMPLETE;
        } else if let Some(caps) = file_change_re.captures(&line) {
            if current_pid != caps[2] {
                return Err(
                    format!("Unexpected PID change from {} to {} on file change line",
                            current_pid, &caps[2])
                );
            }
            total_bytes_sync += caps[4].parse::<i64>().map_err(
                |e| format!("Failed to parse file size {:?} as an i64 integer ({}) on file change line",
                            &caps[4], e.to_string())
            )?;
            total_bytes_sent += caps[5].parse::<i64>().map_err(
                |e| format!("Failed to parse sent size {:?} as an i64 integer ({}) on file change line",
                            &caps[5], e.to_string())
            )?;
            end_time = Some(parse_timestamp(&caps[1]).map_err(
                |e| format!("Failed to parse {:?} as a date ({}) on file change line",
                            &caps[1], e.to_string())
            )?);
            num_files_changed += 1;
        } else if let Some(caps) = connection_closed_re.captures(&line) {
            if current_pid != caps[2] {
                return Err(
                    format!("Unexpected PID change from {} to {} on connection closed line",
                            current_pid, &caps[2])
                );
            }
            end_time = Some(parse_timestamp(&caps[1]).map_err(
                |e| format!("Failed to parse {:?} as a date ({}) on connection closed line",
                            &caps[1], e.to_string())
            )?);
            total_bytes_sent_final = caps[3].parse::<i64>().map_err(
                |e| format!("Failed to parse transfered count {:?} as an i64 integer ({}) on connection closed line",
                            &caps[3], e.to_string())
            )?;
            chunk_parse_error |= ERR_CONNECTION_CLOSED | ERR_CONNECTION_CLOSED_WITOUT_FOLLOW_UP;
        } else if let Some(caps) = signal_received_re.captures(&line) {
            if current_pid != caps[2] {
                return Err(
                    format!("Unexpected PID change from {} to {} on signal received line",
                            current_pid, &caps[2])
                );
            }
            end_time = Some(parse_timestamp(&caps[1]).map_err(
                |e| format!("Failed to parse {:?} as a date ({}) on signal received line",
                            &caps[1], e.to_string())
            )?);
            chunk_parse_error |= ERR_SIGNAL_RECEIVED;
            chunk_parse_error &= !ERR_CHUNK_INCOMPLETE;
        } else {
            return Err(format!("Unexpected log line format"));
        }
    }
    // Ensure the last line is the end line
    if !end_time.is_some() {
        return Err("No lines found in the chunk".to_string());
    }
    let end_time = end_time.unwrap();  // safe to unwrap

    let log = RsyncLog {
        label: label.to_string(),
        start_time: start_time.and_utc().timestamp(),
        end_time: end_time.and_utc().timestamp(),
        total_bytes_sync,
        total_bytes_sent,
        total_bytes_sync_final,
        total_bytes_sent_final,
        num_files_changed,
        chunk_parse_error,
    };

    Ok(log)
}
