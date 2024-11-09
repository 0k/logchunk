use lazy_static::lazy_static;
use std::fs::{File, OpenOptions};
use std::io::copy;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::path::PathBuf;
use std::str;

use chrono::{self, Datelike, TimeZone};
use flate2::read::GzDecoder;
use glob::{glob, GlobError};
use std::str::FromStr;

use std::cell::RefCell;
use std::rc::Rc;

fn mk_iterf<P: AsRef<Path> + Copy>(
    filename: P,
) -> Result<Box<dyn Iterator<Item = Result<String, String>>>, String> {
    let file = File::open(filename)
        .map_err(|e| format!("Error while opening {}: {}", filename.as_ref().display(), e))?;
    Ok(file_mk_iterf(file))
}

fn mk_iterz<P: AsRef<Path> + Copy>(
    filename: P,
) -> Result<Box<dyn Iterator<Item = Result<String, String>>>, String> {
    let file = File::open(filename)
        .map_err(|e| format!("Error while opening {}: {}", filename.as_ref().display(), e))?;
    Ok(file_mk_iterz(file))
}

fn file_mk_iterf<P: std::io::Read + 'static>(
    file: P,
) -> Box<dyn Iterator<Item = Result<String, String>>> {
    Box::new(
        BufReader::new(file)
            .lines()
            .map(|line| line.map_err(|e| e.to_string())),
    )
}

fn file_mk_iterz<P: std::io::Read + 'static>(
    file: P,
) -> Box<dyn Iterator<Item = Result<String, String>>> {
    let gz = GzDecoder::new(file);
    Box::new(
        BufReader::new(gz)
            .lines()
            .map(|line| line.map_err(|e| e.to_string())),
    )
}

fn file_append<P: AsRef<Path>, I: IntoIterator<Item = Result<String, String>>>(
    filename: P,
    lines: I,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(filename)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    for line in lines {
        // line is Results<String, String>
        let line = line.map_err(|e| e.to_string())?;
        writer
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
        writer
            .write_all(b"\n") // add a newline character
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn trim_end_or_error(s: &str, pattern: &str) -> Result<String, String> {
    if s.ends_with(pattern) {
        let new_length = s.len() - pattern.len();
        Ok(s[..new_length].to_string())
    } else {
        Err(format!(
            "The string '{}' does not end with '{}'",
            s, pattern
        ))
    }
}

fn glob_patterns(patterns: Vec<String>) -> Box<dyn Iterator<Item = Result<PathBuf, GlobError>>> {
    let mut iterators: Vec<_> = Vec::new();

    for pattern in patterns {
        match glob(&pattern) {
            Ok(paths) => {
                let mut paths_vec: Vec<_> =
                    paths.collect::<Result<Vec<_>, _>>().unwrap_or_else(|e| {
                        log::debug!("Error collecting glob results: {}", e);
                        Vec::new()
                    });
                paths_vec.sort();
                iterators.push(paths_vec.into_iter().map(Ok));
            }
            Err(e) => {
                log::debug!("Error parsing glob pattern '{}': {}", pattern, e);
            }
        }
    }

    // Flatten the vector of iterators into a single iterator
    let combined = iterators.into_iter().flatten();
    Box::new(combined)
}

fn cursor_file_read(cursor_file: &Path) -> Result<(i64, i64, String, String), String> {
    let mut cts: i64 = 0;
    let mut foffset: i64 = 0;
    let mut cfile = String::new();
    let mut cfile_first_line = String::new();
    if cursor_file.exists() {
        // values are separated by \0
        let file = File::open(cursor_file)
            .map_err(|e| format!("Could not open cursor file {:?}: {}", cursor_file, e))?;
        let mut reader = BufReader::new(file);
        let mut buffer = Vec::new();
        let mut idx = 0;
        while {
            buffer.clear();
            let bytes_read = reader
                .read_until(0x00, &mut buffer)
                .map_err(|e| e.to_string())?;
            // Check if we have reached the end of the file
            bytes_read != 0
        } {
            // Remove the NUL character from the buffer, if it was read
            if let Some(&0x00) = buffer.last() {
                buffer.pop();
            }
            // Convert buffer to UTF-8 string and store the value
            if let Ok(s) = str::from_utf8(&buffer) {
                match idx {
                    0 => {
                        cts = s.parse::<i64>().map_err(|e| {
                            format!("Could not read cursor timestamp data {:?}: {}", s, e)
                        })?
                    }
                    1 => foffset = s.parse::<i64>().map_err(|e| e.to_string())?,
                    2 => cfile = s.to_string(),
                    3 => cfile_first_line = s.to_string(),
                    _ => return Err("Cursor file is corrupted (too many values)".to_string()),
                }
            } else {
                return Err("Bad format".to_string());
            }
            idx += 1;
        }
        if idx != 4 {
            return Err("Cursor file is corrupted (not enough values)".to_string());
        }
    }
    log::trace!("Read from cursor file:");
    log::trace!("  | last_match: {}", cts);
    log::trace!("  | file_offset: {}", foffset);
    log::trace!("  | file: {}", cfile);
    log::trace!("  | first_line: '{}'", cfile_first_line);

    Ok((cts, foffset, cfile, cfile_first_line))
}

fn ts_day_trim(ts: i64) -> Result<i64, String> {
    let dt = chrono::Utc
        .timestamp_opt(ts, 0)
        .single()
        .ok_or("Couldn't parse cursor date")?;
    Ok(chrono::Utc
        .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 0, 0, 0)
        .single()
        .ok_or("Couldn't create datetime")?
        .timestamp())
}

fn cursor_file_write(
    cursor_file: &Path,
    cts: i64,
    foffset: i64,
    cfile: &str,
    cfile_first_line: &str,
) -> Result<(), String> {
    let content = format!("{:?}\0{}\0{}\0{}\0", cts, foffset, cfile, cfile_first_line);
    std::fs::write(cursor_file, content)
        .map_err(|e| format!("Can't write cursor file {}: {}", cursor_file.display(), e))?;
    log::trace!("Stored in cursor:");
    log::trace!("  | last_match: {}", cts);
    log::trace!("  | file_offset: {}", foffset);
    log::trace!("  | file: {}", cfile);
    log::trace!("  | first_line: '{}'", cfile_first_line);

    Ok(())
}

fn file_name_date(file_name: &str) -> Result<Option<i64>, String> {
    let mut basename_log_file_split = file_name.split('_');
    if basename_log_file_split.next().is_none() {
        // ensure there's at least one '_'
        return Ok(None);
    }

    if let Some(log_file_date) = basename_log_file_split.last() {
        // Has a date
        let log_file_date_str = log_file_date.split('.').next().ok_or("Invalid file name")?;
        let file_date_dt = chrono::NaiveDate::parse_from_str(log_file_date_str, "%Y-%m-%d");

        let file_date_dt = if let Ok(dt) = file_date_dt {
            dt
        } else {
            log::trace!("    Invalid date: {:?}", log_file_date_str);
            return Ok(None);
        };

        let file_date_ts = file_date_dt
            .and_hms_opt(0, 0, 0)
            .ok_or("Failed to create date time")?
            .and_utc()
            .timestamp();

        Ok(Some(file_date_ts))
    } else {
        Ok(None)
    }
}

const CHUNK_MAX_PERIOD: i64 = 3 * 24 * 60 * 60; // in seconds

const REGEX_LOG_DATE: &str = r"[0-9]{4,4}/[01][0-9]/[0-3][0-9] [012][0-9]:[0-5][0-9]:[0-5][0-9]";

const REGEX_FIRST_LINE_END: &str = "\\[[0-9]+\\] receiving file list";

lazy_static! {
    static ref RE_FIRST_LINE: Result<regex::Regex, regex::Error> =
        regex::Regex::new(format!("^{REGEX_LOG_DATE} {REGEX_FIRST_LINE_END}$").as_str());
    static ref RE_LAST_LINE: Result<regex::Regex, regex::Error> = regex::Regex::new(
        format!("^{REGEX_LOG_DATE} \\[[0-9]+\\] (sent [0-9]+|rsync error:)").as_str()
    );
    static ref RE_PREFIX_LINE: Result<regex::Regex, regex::Error> = regex::Regex::new(
        format!("^{REGEX_LOG_DATE} \\[[0-9]+\\] ").as_str()
    );
}

pub fn next_chunk(full_file_path: PathBuf, cursor_state_path: PathBuf) -> Result<bool, String> {
    // Is the log file name is correctly formatted ?

    let log_file_stem = full_file_path
        .to_str()
        .ok_or_else(|| "Path contains invalid Unicode".to_string())?;
    let log_file_stem = trim_end_or_error(
        log_file_stem,
        ".log",
    )
    .map_err(|_e| {
        format!("Unexpected extension to target log file name {:?} (it is required to end with '.log')",
        log_file_stem)
    })?
    .to_string();

    if !cursor_state_path.exists() {
        std::fs::create_dir_all(&cursor_state_path).map_err(|e| {
            format!(
                "Couldn't create cursor state directory {:?}.\n  {}",
                cursor_state_path, e
            )
        })?;
    }
    let content_file_path = cursor_state_path.join("content");
    let cursor_file = cursor_state_path.join("cursor");

    let (cts, mut foffset, mut cfile, cfile_first_line) = cursor_file_read(&cursor_file)?;

    // Was the file the cursor is pointing to, rotated ?

    let mut file_rotated = if !cfile.is_empty() {
        let reader = if PathBuf::from_str(&cfile)
            .map_err(|e| e.to_string())?
            .extension()
            .map_or(false, |ext| ext == "gz")
        {
            mk_iterz
        } else {
            mk_iterf
        };
        let first_line = reader(&cfile)?.next();

        let first_line = if let Some(first_line) = first_line {
            first_line.map_err(|e| {
                format!(
                    "Error while reading first line from {}\n  {}",
                    cfile,
                    e.to_string()
                )
            })?
        } else {
            "".to_string()
        };
        if cfile_first_line != first_line {
            log::debug!("File '{}' rotated", cfile);
        }
        cfile_first_line != first_line
    } else {
        false
    };

    let cts_day_trimmed = ts_day_trim(cts)?;

    log::debug!("cts: '{}' '{}' trimmed: {}", cts, cfile, cts_day_trimmed);
    let mut first_match_line_nb = 0;

    let mut log_file_paths = Vec::new();
    for log_file in glob_patterns(vec![
        format!("{log_file_stem}_[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9].log.gz"),
        format!("{log_file_stem}_[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9].log"),
        format!("{log_file_stem}.log"),
    ]) {
        let log_file_path = log_file.map_err(|e| format!("Error: {e}").to_string())?;
        let basename_log_file = log_file_path.file_name().ok_or(format!(
            "Couldn't get base name of {}",
            log_file_path.display()
        ))?;
        let basename_log_file = basename_log_file
            .to_str()
            .ok_or("Invalid Unicode in file name")?;
        let mut basename_log_file_split = basename_log_file.split('_');
        basename_log_file_split.next();

        if let Some(file_date_ts) = file_name_date(basename_log_file)? {
            if file_date_ts < cts_day_trimmed {
                log::debug!(
                    "  Skipping file '{}' by date of file ({:?} < cursor)",
                    basename_log_file,
                    file_date_ts
                );
                continue;
            }
            log::trace!(
                "  date found in file name {:?} is after cursor",
                basename_log_file
            );
        } else {
            log::trace!(
                "  no date found in file name {:?}, using it !",
                basename_log_file
            );
        }
        log_file_paths.push((log_file_path.clone(), basename_log_file.to_string()));
    }

    for (log_file_path, basename_log_file) in log_file_paths.into_iter() {
        log::trace!("Considering: {}", basename_log_file);

        let reader = if log_file_path.extension().map_or(false, |ext| ext == "gz") {
            mk_iterz
        } else {
            mk_iterf
        };

        let log_file_name = log_file_path
            .to_str()
            .ok_or("Invalid Unicode in file name")?;

        if file_rotated {
            let first_line_opt = reader(&log_file_path)?.next();
            match first_line_opt {
                Some(first_line) if *first_line.as_ref()? == cfile_first_line => {
                    log::debug!(
                        "  Our file '{}' was rotated to '{}'",
                        cfile,
                        basename_log_file
                    );
                    Ok(true)
                }
                Some(first_line) => {
                    log::error!(
                        indoc::indoc! {"
                        Lost our previous reference !
                          Our state file recorded a file starting with:
                              {}
                            But the new chronological file '{}' starts with:
                              {}"},
                        cfile_first_line,
                        basename_log_file,
                        first_line?
                    );
                    Err("Lost our previous reference !")
                }
                _ => {
                    log::error!(
                        indoc::indoc! {"
                        Lost our previous reference !
                          Our state file recorded a file starting with:
                              {}
                            But the new chronological file '{}' seems empty"},
                        cfile_first_line,
                        basename_log_file,
                    );
                    Err("Lost our previous reference !")
                }
            }?;
            cfile = log_file_name.to_string();
            file_rotated = false;
        };

        let mut first_line = String::new();

        if content_file_path.exists() {
            first_line = BufReader::new(File::open(&content_file_path).map_err(|e| e.to_string())?)
                .lines()
                .next()
                .ok_or("Couldn't read first line from content file")?
                .map_err(|e| e.to_string())?;
            first_match_line_nb = 1;
        } else if first_match_line_nb == 0 {
            let offset = if log_file_name == cfile { foffset } else { 0 };
            log::trace!(
                "Looking for beginning of chunk in '{}' (offset: {}).",
                basename_log_file,
                offset
            );
            let re_first_line = RE_FIRST_LINE.as_ref().map_err(|e| e.to_string())?;
            let first_match_line_match = reader(&log_file_path)?
                .skip(offset as usize)
                .enumerate()
                .find_map(|(line_nb, line_res)| match line_res {
                    Ok(line) => re_first_line.is_match(&line).then(|| Ok((line_nb, line))),
                    Err(err) => Some(Err(err)),
                });
            (first_match_line_nb, first_line) = match first_match_line_match {
                Some(Ok((line_nb, line))) => (line_nb, line),
                Some(Err(e)) => return Err(e),
                None => {
                    log::debug!(
                        "Skipping file '{}': no beginning of chunk.",
                        basename_log_file
                    );
                    continue;
                }
            };

            if first_match_line_nb != 0 {
                // get the first line of the file
                let prefix = reader(&log_file_path)?
                    .skip(offset as usize)
                    .take(first_match_line_nb as usize + 1)
                    .collect::<Result<Vec<String>, String>>()?
                    .join("\n");
                return Err(format!("Unexpected {} lines before beg line:\n{}",
                                      first_match_line_nb,
                                   prefix));

            }
            log::trace!("  found beg line at: {}", first_match_line_nb);
            log::trace!("    line: {}", first_line);
            log::trace!("    offset: {}", offset);
            foffset = first_match_line_nb as i64 + offset;
            log::trace!("    new foffset: {}", foffset);
            cfile = log_file_path.display().to_string();
        }

        if log_file_name != cfile {
            foffset = 0;
        }

        // Extract pid from first_line
        //   firt_line:
        //   2023/02/27 02:28:40 [12759] >f..t...... recv weave/failed/forms.json 10 34
        let first_line_parts = first_line.split(' ').collect::<Vec<&str>>();
        let pid = first_line_parts
            .get(2)
            .ok_or("Couldn't extract pid from first line")?
            .trim_matches(|c| c == '[' || c == ']');

        // Extract date and time from first_line
        //   first_line:
        //   2023/02/27 02:28:40 [12759] >f..t...... recv weave/failed/forms.json 10 34

        let first_match_datetime = first_line_parts
            .iter()
            .take(2)
            .copied()
            .collect::<Vec<&str>>()
            .join(" ");

        // Convert date to timestamp
        let first_match_ts = chrono::NaiveDateTime::parse_from_str(
            first_match_datetime.as_str(),
            "%Y/%m/%d %H:%M:%S",
        )
        .map_err(|e| format!("Couldn't parse date from {}: {}", first_match_datetime, e))?
        .and_utc()
        .timestamp();

        log::trace!(
            "  first match datetime: {}",
            first_match_datetime.to_string()
        );

        let max_period_ts = first_match_ts + CHUNK_MAX_PERIOD;
        let max_period_datetime = chrono::Utc
            .timestamp_opt(max_period_ts, 0)
            .single()
            .ok_or("Couldn't parse max period date")?
            .format("%F %T")
            .to_string();

        log::trace!("  max period datetime: {}", max_period_datetime);

        log::info!(
            "Looking for end of chunk in {}:{}",
            basename_log_file,
            foffset
        );
        let re_last_line = RE_LAST_LINE.as_ref().map_err(|e| e.to_string())?;
        let re_prefix_line = RE_PREFIX_LINE.as_ref().map_err(|e| e.to_string())?;
        let last_candidate_line = Rc::new(RefCell::new(first_line.clone()));
        let last_candidate_idx = Rc::new(RefCell::new(first_match_line_nb));


        let last_match_line_nb = reader(&log_file_path)?
            .skip(foffset as usize)
            .enumerate()
            .find_map(|(line_nb, line_res)|
                      match line_res {
                Ok(line) => {
                    if re_prefix_line.is_match(&line) {
                        let candidate_line_parts = line.split(' ').collect::<Vec<&str>>();
                        let candidate_pid = candidate_line_parts
                            .get(2)
                            .ok_or("Couldn't extract pid from candidate line");
                        let candidate_pid = match candidate_pid {
                            Ok(pid) => pid,
                            Err(e) => return Some(Err(e.to_string())),
                        };
                        let candidate_pid = candidate_pid
                            .trim_matches(|c| c == '[' || c == ']');
                        if candidate_pid == pid {
                            *last_candidate_line.borrow_mut() = line.clone();
                            *last_candidate_idx.borrow_mut() = line_nb;
                            return re_last_line.is_match(&line).then(|| Ok((line_nb, line)))
                        }
                        log::trace!("  Ending chunk at line with different pid: {}", line);
                    } else {
                        log::trace!("  Ending chunk at line with no prefix: {}", line);
                    }
                    return Some(Ok((*last_candidate_idx.borrow(), (*last_candidate_line.borrow()).to_string())))
                },
                Err(err) => Some(Err(err)),
            });
        match last_match_line_nb {
            None => {
                log::debug!("  Finished {} with no end of chunk.", basename_log_file);

                let content_line_nb = if content_file_path.exists() {
                    let content_line_nb =
                        BufReader::new(File::open(&content_file_path).map_err(|e| e.to_string())?)
                            .lines()
                            .count() as i64;
                    log::trace!("  content file exists");
                    content_line_nb
                } else {
                    log::trace!("  no content file yet");
                    0
                };
                log::trace!("  content_line_nb: {}", content_line_nb);

                file_append(
                    &content_file_path,
                    reader(&log_file_path)?.skip(foffset as usize),
                )?;

                // log current content file with a prefix
                log::trace!(
                    "CONTENT:\n  | {}",
                    std::fs::read_to_string(&content_file_path)
                        .map_err(|e| e.to_string())?
                        .replace("\n", "\n  | ")
                );

                let new_content_line_nb =
                    BufReader::new(File::open(&content_file_path).map_err(|e| e.to_string())?)
                        .lines()
                        .count() as i64;

                log::trace!("  New content_line_nb: {}", new_content_line_nb);
                log::trace!("  Previous offset: {}", foffset);

                foffset += new_content_line_nb - content_line_nb;

                log::trace!("  New offset: {}", foffset);
                log::trace!(
                    "  Stored in content:\n  | {}",
                    std::fs::read_to_string(&content_file_path)
                        .map_err(|e| e.to_string())?
                        .replace("\n", "\n  | ")
                );
                continue;
            }
            Some(Err(e)) => return Err(e),
            Some(Ok((last_match_line_nb, last_line))) => {
                log::trace!("  found end line at: {}", last_match_line_nb);
                log::trace!("  line: {}", last_line);
                let last_match_line_nb = last_match_line_nb as i64;

                // Extract date and time from last_line
                //   last_line:
                //   2023/02/27 02:28:40 [12759] >f..t...... recv weave/failed/forms.json 10 34

                let last_match_datetime = last_line
                    .split(' ')
                    .take(2)
                    .collect::<Vec<&str>>()
                    .join(" ");
                // Convert date to timestamp
                let last_match_ts = chrono::NaiveDateTime::parse_from_str(
                    last_match_datetime.as_str(),
                    "%Y/%m/%d %H:%M:%S",
                )
                .map_err(|e| format!("Couldn't parse date from {}: {}", first_match_datetime, e))?
                .and_utc()
                .timestamp();

                log::trace!(
                    "{}: pid: {} lines: {} -> {}",
                    basename_log_file,
                    pid,
                    foffset,
                    foffset + last_match_line_nb
                );

                if content_file_path.exists() {
                    let content = File::open(&content_file_path).map_err(|e| e.to_string())?;
                    let mut reader = BufReader::new(content);
                    let mut stdout = std::io::stdout();
                    copy(&mut reader, &mut stdout).map_err(|e| e.to_string())?;
                    std::fs::remove_file(&content_file_path).map_err(|e| e.to_string())?;
                }

                log::trace!("  SED {}:{}", foffset, foffset + last_match_line_nb);

                let lines = reader(&log_file_path)?
                    .skip(foffset as usize)
                    .take((last_match_line_nb + 1) as usize);

                for line in lines {
                    let line = line.map_err(|e| format!("Can't read line: {e}"))?;
                    writeln!(std::io::stdout(), "{}", line)
                        .map_err(|e| format!("Can't write line: {}", e))?;
                }
                let first_line = reader(&log_file_path)?
                    .next()
                    .ok_or("Couldn't read first line from file")?
                    .map_err(|e| e.to_string())?;

                cursor_file_write(
                    &cursor_file,
                    last_match_ts,
                    foffset + last_match_line_nb + 1,
                    log_file_name,
                    &first_line,
                )?;

                return Ok(true);
            }
        };
    }

    // This code will be reached only if all log files were read
    // Thus,

    // YYYvlab: is file empty ?
    let first_line = if let Some(first_line) =
        BufReader::new(File::open(&full_file_path).map_err(|e| {
            format!(
                "Error opening {}: {}",
                full_file_path.display(),
                e.to_string()
            )
        })?)
        .lines()
        .next()
    {
        first_line.map_err(|e| e.to_string())?
    } else {
        "".to_string()
    };

    // get last line of content file
    let new_cts = if content_file_path.exists() {
        let content_last_line = BufReader::new(File::open(&content_file_path).map_err(|e| {
            format!(
                "Error opening {}: {}",
                content_file_path.display(),
                e.to_string()
            )
        })?)
        .lines()
        .last()
        .ok_or("Couldn't read last line from content file")?
        .map_err(|e| e.to_string())?;

        let content_last_datetime = content_last_line
            .split(' ')
            .take(2)
            .collect::<Vec<&str>>()
            .join(" ");

        chrono::NaiveDateTime::parse_from_str(&content_last_datetime.as_str(), "%Y/%m/%d %H:%M:%S")
            .map_err(|e| format!("Couldn't parse date from {}: {}", content_last_datetime, e))?
            .and_utc()
            .timestamp()
    } else {
        cts
    };

    cursor_file_write(
        &cursor_file,
        new_cts,
        foffset,
        full_file_path
            .to_str()
            .ok_or("Invalid Unicode in file name")?,
        first_line.as_str(),
    )?;

    log::trace!("Didn't find any full entry");
    Ok(false)
}
