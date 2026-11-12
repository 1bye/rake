//! Bounded, unassociated process output shared by owned pipe readers and the supervisor.

use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

pub(super) struct ProcessOutput {
    records: VecDeque<Value>,
    sequence: u64,
    dropped: u64,
    started: Instant,
}

pub(super) type SharedOutput = Arc<Mutex<ProcessOutput>>;

impl ProcessOutput {
    pub(super) fn new() -> SharedOutput {
        Arc::new(Mutex::new(Self {
            records: VecDeque::new(),
            sequence: 0,
            dropped: 0,
            started: Instant::now(),
        }))
    }

    pub(super) fn append(
        &mut self,
        text: &str,
        pid: u32,
        stream: &str,
        level: Option<&str>,
        structured: bool,
    ) {
        if self.records.len() == 128 {
            self.records.pop_front();
            self.dropped += 1;
        }
        self.sequence += 1;
        let mut end = text.len().min(2048);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.records.push_back(
            json!({"sequence":self.sequence, "elapsed_us":self.started.elapsed().as_micros() as u64,
            "pid":pid, "stream":stream, "level":level, "structured":structured,
            "text":&text[..end], "truncated":text.len() > 2048}),
        );
    }

    pub(super) fn snapshot(&self) -> Value {
        json!({"records":self.records, "dropped":self.dropped, "revision":self.sequence})
    }
}

pub(super) fn collect(
    pipe: impl Read + Send + 'static,
    output: SharedOutput,
    pid: u32,
    stream: &'static str,
) -> JoinHandle<std::result::Result<(), String>> {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        loop {
            let text = read_line(&mut reader).map_err(|error| error.to_string())?;
            if text.is_empty() {
                break;
            }
            let (text, structured) = text
                .strip_prefix("\x1eRAKE_DIAGNOSTIC ")
                .map_or((text.as_str(), false), |text| (text, true));
            output
                .lock()
                .expect("process output lock poisoned")
                .append(text, pid, stream, None, structured);
            let mut stderr = std::io::stderr().lock();
            stderr
                .write_all(text.as_bytes())
                .map_err(|error| error.to_string())?;
            stderr.flush().map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

// Match TextIOWrapper(errors="replace", newline=None).readline(2048), including
// multibyte characters crossing a pipe read and the structured diagnostic prefix.
fn read_line(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut text = String::new();
    for _ in 0..2048 {
        let Some(&first) = reader.fill_buf()?.first() else {
            break;
        };
        reader.consume(1);
        if first == b'\r' {
            if reader.fill_buf()?.first() == Some(&b'\n') {
                reader.consume(1);
            }
            text.push('\n');
            break;
        }
        if first < 128 {
            text.push(first as char);
            if first == b'\n' {
                break;
            }
            continue;
        }
        let length = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        };
        let mut encoded = vec![first];
        for index in 1..length {
            let Some(&byte) = reader.fill_buf()?.first() else {
                break;
            };
            let valid = (0x80..=0xbf).contains(&byte)
                && (index != 1
                    || match first {
                        0xe0 => byte >= 0xa0,
                        0xed => byte <= 0x9f,
                        0xf0 => byte >= 0x90,
                        0xf4 => byte <= 0x8f,
                        _ => true,
                    });
            if !valid {
                break;
            }
            reader.consume(1);
            encoded.push(byte);
        }
        if length > 0 && encoded.len() == length {
            text.push_str(&String::from_utf8_lossy(&encoded));
        } else {
            text.push('\u{fffd}');
        }
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    //! Covers pipe decoding and Python's character (rather than byte) line cap.
    //! Real: bounded output collector. Doubles: in-memory pipe bytes. Requires: none.
    use super::*;

    #[test]
    fn decoding_preserves_multibyte_characters_line_bounds_and_invalid_utf8() {
        let input = format!("{}z\r\nlast\rnext", "漢".repeat(2048));
        let mut reader = BufReader::with_capacity(2, input.as_bytes());
        assert_eq!(read_line(&mut reader).unwrap(), "漢".repeat(2048));
        assert_eq!(read_line(&mut reader).unwrap(), "z\n");
        assert_eq!(read_line(&mut reader).unwrap(), "last\n");
        assert_eq!(read_line(&mut reader).unwrap(), "next");
        assert_eq!(read_line(&mut reader).unwrap(), "");
        let mut reader = BufReader::new(&b"bad \xe2\x82X\xff\r\n"[..]);
        assert_eq!(read_line(&mut reader).unwrap(), "bad �X�\n");
    }
}
