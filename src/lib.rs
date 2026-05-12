use std::cmp::min;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

const READER_CAPACITY: usize = 1024 * 1024;
const TAIL_CHUNK_SIZE: u64 = 64 * 1024;
const RANDOM_BACKSCAN_CHUNK: u64 = 8 * 1024;
const RANDOM_FAST_ATTEMPTS_MULTIPLIER: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputOptions {
    pub pretty: Option<usize>,
    pub max_chars: usize,
    pub show_line_numbers: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedLine {
    pub line_number: Option<usize>,
    pub content: String,
}

pub fn read_head(path: &Path, count: usize) -> Result<Vec<SelectedLine>> {
    read_range(path, 0, count)
}

pub fn read_range(path: &Path, start: usize, count: usize) -> Result<Vec<SelectedLine>> {
    ensure_positive("count", count)?;

    let file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let mut reader = BufReader::with_capacity(READER_CAPACITY, file);
    let mut line = String::new();
    let mut current = 0usize;
    let mut entries = Vec::with_capacity(count);

    while entries.len() < count {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        let zero_based = current;
        current += 1;
        if zero_based < start {
            continue;
        }

        entries.push(SelectedLine {
            line_number: Some(current),
            content: trim_line_ending(&line).to_owned(),
        });
    }

    Ok(entries)
}

pub fn read_tail(path: &Path, count: usize) -> Result<Vec<SelectedLine>> {
    ensure_positive("count", count)?;

    let mut file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(Vec::new());
    }

    let mut pos = file_len;
    let mut buffer = Vec::new();
    let mut newline_count = 0usize;

    while pos > 0 && newline_count <= count {
        let read_size = min(TAIL_CHUNK_SIZE, pos) as usize;
        pos -= read_size as u64;
        file.seek(SeekFrom::Start(pos))?;

        let mut chunk = vec![0u8; read_size];
        file.read_exact(&mut chunk)?;
        newline_count += chunk.iter().filter(|&&byte| byte == b'\n').count();

        chunk.extend_from_slice(&buffer);
        buffer = chunk;
    }

    let slice = if starts_at_line_boundary(&mut file, pos)? {
        &buffer[..]
    } else {
        match buffer.iter().position(|&byte| byte == b'\n') {
            Some(index) => &buffer[index + 1..],
            None => &buffer[..],
        }
    };

    let text = String::from_utf8_lossy(slice);
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end_matches('\r').to_owned())
        .collect();

    if lines.len() > count {
        lines = lines.split_off(lines.len() - count);
    }

    Ok(lines
        .into_iter()
        .map(|content| SelectedLine {
            line_number: None,
            content,
        })
        .collect())
}

pub fn count_lines(path: &Path) -> Result<usize> {
    let file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let mut reader = BufReader::with_capacity(READER_CAPACITY, file);
    let mut line = String::new();
    let mut count = 0usize;

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        count += 1;
    }

    Ok(count)
}

pub fn attach_tail_line_numbers(path: &Path, entries: &mut [SelectedLine]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let total = count_lines(path)?;
    let start = total.saturating_sub(entries.len()) + 1;
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.line_number = Some(start + index);
    }
    Ok(())
}

pub fn read_random_fast(path: &Path, count: usize) -> Result<Vec<SelectedLine>> {
    ensure_positive("count", count)?;

    let mut file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let file_len = file.metadata()?.len();
    if file_len == 0 {
        return Ok(Vec::new());
    }

    let mut seen_starts = HashSet::with_capacity(count.saturating_mul(2));
    let mut sampled = Vec::with_capacity(count);
    let attempts = count
        .saturating_mul(RANDOM_FAST_ATTEMPTS_MULTIPLIER)
        .max(count);

    for _ in 0..attempts {
        if sampled.len() >= count {
            break;
        }

        let offset = rand::random_range(0..file_len);
        if let Some((line_start, content)) = read_line_covering_offset(&mut file, file_len, offset)?
        {
            if seen_starts.insert(line_start) {
                sampled.push((
                    line_start,
                    SelectedLine {
                        line_number: None,
                        content,
                    },
                ));
            }
        }
    }

    if sampled.len() < count {
        return read_random_buffered(path, count);
    }

    sampled.sort_by_key(|(line_start, _)| *line_start);
    Ok(sampled.into_iter().map(|(_, entry)| entry).collect())
}

pub fn read_random_buffered(path: &Path, count: usize) -> Result<Vec<SelectedLine>> {
    ensure_positive("count", count)?;

    let file =
        File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let mut reader = BufReader::with_capacity(READER_CAPACITY, file);
    let mut line = String::new();
    let mut reservoir = Vec::with_capacity(count);
    let mut seen = 0usize;

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        seen += 1;
        let entry = SelectedLine {
            line_number: Some(seen),
            content: trim_line_ending(&line).to_owned(),
        };

        if reservoir.len() < count {
            reservoir.push(entry);
            continue;
        }

        let slot = rand::random_range(0..seen);
        if slot < count {
            reservoir[slot] = entry;
        }
    }

    reservoir.sort_by_key(|entry| entry.line_number.unwrap_or(usize::MAX));
    Ok(reservoir)
}

fn read_line_covering_offset(
    file: &mut File,
    file_len: u64,
    offset: u64,
) -> Result<Option<(u64, String)>> {
    if file_len == 0 {
        return Ok(None);
    }

    let mut effective_offset = offset.min(file_len - 1);
    if effective_offset > 0 {
        file.seek(SeekFrom::Start(effective_offset))?;
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            effective_offset -= 1;
        }
    }

    let line_start = find_line_start(file, effective_offset)?;
    file.seek(SeekFrom::Start(line_start))?;

    let mut reader = BufReader::with_capacity(READER_CAPACITY, file);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Ok(None);
    }

    Ok(Some((line_start, trim_line_ending(&line).to_owned())))
}

fn find_line_start(file: &mut File, offset: u64) -> Result<u64> {
    if offset == 0 {
        return Ok(0);
    }

    let mut scan_end = offset;
    let mut buffer = vec![0u8; RANDOM_BACKSCAN_CHUNK as usize];

    loop {
        let chunk_start = scan_end.saturating_sub(RANDOM_BACKSCAN_CHUNK - 1);
        let read_len = (scan_end - chunk_start + 1) as usize;
        file.seek(SeekFrom::Start(chunk_start))?;
        file.read_exact(&mut buffer[..read_len])?;

        if let Some(pos) = buffer[..read_len].iter().rposition(|&byte| byte == b'\n') {
            return Ok(chunk_start + pos as u64 + 1);
        }

        if chunk_start == 0 {
            return Ok(0);
        }

        scan_end = chunk_start - 1;
    }
}

pub fn render_lines(entries: &[SelectedLine], options: &OutputOptions) -> Result<String> {
    let mut output = String::new();
    let pretty_gap = options.pretty.unwrap_or(0);
    let pretty_enabled = options.pretty.is_some();

    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            output.push('\n');
            for _ in 0..pretty_gap {
                output.push('\n');
            }
        }

        if pretty_enabled {
            if options.show_line_numbers {
                match entry.line_number {
                    Some(line_number) => {
                        output.push_str(&format!("[line {line_number}]\n"));
                    }
                    None => {
                        output.push_str(&format!("[entry {}]\n", index + 1));
                    }
                }
            }

            match format_json_line(&entry.content, options.max_chars) {
                Ok(formatted) => output.push_str(&formatted),
                Err(_) => {
                    output.push_str(&entry.content);
                }
            }
        } else {
            if options.show_line_numbers {
                match entry.line_number {
                    Some(line_number) => {
                        output.push_str(&format!("[line {line_number}] {}", entry.content));
                    }
                    None => {
                        output.push_str(&entry.content);
                    }
                }
            } else {
                output.push_str(&entry.content);
            }
        }
    }

    if !output.is_empty() {
        output.push('\n');
    }

    Ok(output)
}

fn format_json_line(line: &str, max_chars: usize) -> Result<String> {
    let mut value: Value = serde_json::from_str(line)?;
    if max_chars > 0 {
        truncate_strings(&mut value, max_chars);
    }
    Ok(serde_json::to_string_pretty(&value)?)
}

fn truncate_strings(value: &mut Value, max_chars: usize) {
    match value {
        Value::String(text) => {
            if text.chars().count() > max_chars {
                let truncated: String = text.chars().take(max_chars).collect();
                *text = format!("{truncated}...");
            }
        }
        Value::Array(items) => {
            for item in items {
                truncate_strings(item, max_chars);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                truncate_strings(value, max_chars);
            }
        }
        _ => {}
    }
}

fn trim_line_ending(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

fn starts_at_line_boundary(file: &mut File, pos: u64) -> Result<bool> {
    if pos == 0 {
        return Ok(true);
    }

    file.seek(SeekFrom::Start(pos - 1))?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    Ok(byte[0] == b'\n')
}

fn ensure_positive(name: &str, value: usize) -> Result<()> {
    if value == 0 {
        bail!("{name} must be greater than 0");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        OutputOptions, SelectedLine, attach_tail_line_numbers, count_lines, read_random_fast,
        read_range, read_tail, render_lines, truncate_strings,
    };

    #[test]
    fn read_range_uses_one_based_indices() {
        let dir = tempdir().unwrap();
        let path = write_file(dir.path(), "sample.jsonl", "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n");

        let entries = read_range(&path, 1, 2).unwrap();

        assert_eq!(
            entries,
            vec![
                SelectedLine {
                    line_number: Some(2),
                    content: "{\"a\":2}".to_owned(),
                },
                SelectedLine {
                    line_number: Some(3),
                    content: "{\"a\":3}".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn read_tail_returns_last_lines_without_full_scan() {
        let dir = tempdir().unwrap();
        let path = write_file(dir.path(), "tail.jsonl", "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n");

        let entries = read_tail(&path, 2).unwrap();

        assert_eq!(
            entries,
            vec![
                SelectedLine {
                    line_number: None,
                    content: "{\"a\":2}".to_owned(),
                },
                SelectedLine {
                    line_number: None,
                    content: "{\"a\":3}".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn truncate_strings_shortens_long_json_values() {
        let mut value = serde_json::json!({
            "name": "abcdefghijklmnopqrstuvwxyz",
            "nested": {
                "path": "1234567890"
            }
        });

        truncate_strings(&mut value, 5);

        assert_eq!(value["name"], "abcde...");
        assert_eq!(value["nested"]["path"], "12345...");
    }

    #[test]
    fn render_lines_pretty_formats_json() {
        let entries = vec![SelectedLine {
            line_number: Some(7),
            content: "{\"name\":\"abcdefghijklmnopqrstuvwxyz\"}".to_owned(),
        }];
        let options = OutputOptions {
            pretty: Some(0),
            max_chars: 4,
            show_line_numbers: true,
        };

        let rendered = render_lines(&entries, &options).unwrap();

        assert!(rendered.contains("[line 7]"));
        assert!(rendered.contains("\"name\": \"abcd...\""));
    }

    #[test]
    fn render_lines_pretty_with_gap_inserts_blank_line() {
        let entries = vec![
            SelectedLine {
                line_number: Some(1),
                content: "{\"a\":1}".to_owned(),
            },
            SelectedLine {
                line_number: Some(2),
                content: "{\"a\":2}".to_owned(),
            },
        ];
        let options = OutputOptions {
            pretty: Some(1),
            max_chars: 10,
            show_line_numbers: false,
        };

        let rendered = render_lines(&entries, &options).unwrap();

        assert!(rendered.contains("}\n\n{\n"));
    }

    #[test]
    fn count_lines_counts_every_jsonl_row() {
        let dir = tempdir().unwrap();
        let path = write_file(dir.path(), "count.jsonl", "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n");

        assert_eq!(count_lines(&path).unwrap(), 3);
    }

    #[test]
    fn attach_tail_line_numbers_sets_final_indices() {
        let dir = tempdir().unwrap();
        let path = write_file(dir.path(), "tailnum.jsonl", "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n");
        let mut entries = read_tail(&path, 2).unwrap();

        attach_tail_line_numbers(&path, &mut entries).unwrap();

        assert_eq!(entries[0].line_number, Some(2));
        assert_eq!(entries[1].line_number, Some(3));
    }

    #[test]
    fn read_random_fast_returns_requested_number_of_lines() {
        let dir = tempdir().unwrap();
        let path = write_file(
            dir.path(),
            "random-fast.jsonl",
            "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n{\"a\":4}\n",
        );

        let entries = read_random_fast(&path, 2).unwrap();

        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert!(entry.content.starts_with("{\"a\":"));
        }
    }

    fn write_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }
}
