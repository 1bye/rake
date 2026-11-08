//! Covers: capture packet bounds, truncation, metadata and epoch preservation.
//! Real: binary codec. Doubles: packet buffers and subprocess helpers. Requires: Unix.
use super::*;
use serde_json::json;

#[test]
fn truncated_and_oversized_capture_frames_fail_explicitly() {
    assert!(read_packet(&mut b"x".as_slice(), "epoch").is_err());

    for sizes in [(0u32, 1u32), (4097, 1), (1, 0), (1, 2 * 1024 * 1024 + 1)] {
        let mut wire = Vec::new();

        wire.extend_from_slice(&sizes.0.to_be_bytes());
        wire.extend_from_slice(&sizes.1.to_be_bytes());
        assert!(read_packet(&mut wire.as_slice(), "epoch").is_err());
    }

    let wire = packet(
        &json!({"kind":"frame","width":600,"height":400,"key":true}),
        b"payload",
    );

    let (metadata, encoded) = read_packet(&mut wire.as_slice(), "epoch").unwrap();

    assert_eq!(metadata["epoch"], "epoch");
    assert!(encoded.ends_with(b"payload"));

    for invalid in [
        json!({"kind":"frame","width":true,"height":400}),
        json!({"kind":"frame","width":4097,"height":1}),
        json!({"kind":"invalid","width":1,"height":1}),
    ] {
        assert!(read_packet(&mut packet(&invalid, b"x").as_slice(), "epoch").is_err());
    }
}

fn helper(root: &Path, packets: &[u8]) -> Command {
    std::fs::write(root.join("packets"), packets).unwrap();
    let helper = root.join("helper");

    std::fs::write(&helper,format!("#!/bin/sh\n/bin/dd bs=1 count=1 of=/dev/null 2>/dev/null\n/bin/cat '{}'\nexec /bin/sleep 300\n",root.join("packets").display())).unwrap();
    // Interpret new fixture files instead of executing them: concurrent forks can
    // briefly inherit a writer and cause Linux ETXTBSY despite CLOEXEC.
    let mut command = Command::new("/bin/sh");

    command.arg(helper);

    command
}

#[test]
fn video_join_starts_at_keyframe_and_releases_viewer_on_disconnect() {
    let temp = tempfile::tempdir().unwrap();
    let mut packets = Vec::new();

    for (kind, key, payload) in [
        ("config", false, "config"),
        ("frame", false, "old-delta"),
        ("frame", true, "key"),
        ("frame", false, "delta"),
    ] {
        packets.extend(packet(
            &json!({"kind":kind,"key":key,"width":600,"height":400}),
            payload.as_bytes(),
        ));
    }

    let capture = Arc::new(Capture::spawn(helper(temp.path(), &packets), "epoch").unwrap());
    let viewer = capture.viewer();

    struct Writer {
        writes: Vec<Vec<u8>>,
    }

    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.writes.push(bytes.to_vec());

            if self.writes.len() == 3 {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(bytes.len())
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer { writes: Vec::new() };

    assert!(viewer.stream(&mut writer).is_err());
    drop(viewer);
    assert_eq!(capture.viewers(), 0);
    assert_eq!(writer.writes.len(), 3);

    for (bytes, expected) in writer.writes.iter().zip(["config", "key", "delta"]) {
        assert!(bytes.ends_with(expected.as_bytes()));
    }

    capture.close();
}

#[test]
fn closing_capture_reaps_its_helper_and_releases_waiting_viewers() {
    let temp = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::spawn(helper(temp.path(), &[]), "epoch").unwrap());

    let pid = capture
        .shared
        .process
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .0
        .id();

    let viewer = capture.viewer();

    capture.close();
    assert!(viewer.stream(&mut Vec::new()).is_ok());
    drop(viewer);
    assert_eq!(capture.viewers(), 0);
    // SAFETY: signal zero observes the helper owned by this fixture.
    assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0);
    assert!(capture.shared.buffer.lock().unwrap().closed);
}

#[test]
fn capture_queue_bounds_both_packet_count_and_retained_bytes() {
    for payload_size in [1, 2 * 1024 * 1024] {
        let temp = tempfile::tempdir().unwrap();
        let count = if payload_size == 1 { 150 } else { 5 };

        let encoded = packet(
            &json!({"kind":"frame","width":600,"height":400,"key":true}),
            &vec![0; payload_size],
        );

        let packets = encoded.repeat(count);
        let capture = Capture::spawn(helper(temp.path(), &packets), "epoch").unwrap();

        capture.shared.keyframe();
        let buffer = capture.shared.buffer.lock().unwrap();

        let (buffer, deadline) = capture
            .shared
            .changed
            .wait_timeout_while(buffer, Duration::from_secs(5), |buffer| {
                buffer.sequence < count as u64 && buffer.error.is_none()
            })
            .unwrap();

        assert!(!deadline.timed_out());
        assert_eq!(buffer.sequence, count as u64);
        assert!(buffer.error.is_none(), "{:?}", buffer.error);
        assert!(buffer.packets.len() <= 120);
        assert!(buffer.bytes <= 8 * 1024 * 1024);
        drop(buffer);
        capture.close();
    }
}
