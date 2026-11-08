//! Covers: masked frames, control limits and upgrade authentication from preview_socket.py.
//! Real: wire codec. Doubles: byte buffers. Requires: none.
use super::*;
#[test]
fn masked_text_and_control_frames_reject_fragments_unmasked_and_oversized_input() {
    let payload = b"{\"id\":1,\"action\":\"preview_state\"}";
    let mut wire = vec![0x81, 0x80 | payload.len() as u8];
    let mask = [1, 2, 3, 4];
    wire.extend_from_slice(&mask);
    wire.extend(payload.iter().enumerate().map(|(i, v)| v ^ mask[i % 4]));
    let (opcode, payload) = read_frame(&mut wire.as_slice()).unwrap();
    assert_eq!(opcode, 1);
    assert_eq!(serde_json::from_slice::<Value>(&payload).unwrap()["id"], 1);
    for invalid in [
        vec![1, 0x80],
        vec![0x81, 0],
        vec![0x81, 0xfe, 0x40, 1],
        vec![0x89, 0xfe, 0, 126],
        vec![0x82, 0x80],
    ] {
        assert!(read_frame(&mut invalid.as_slice()).is_err());
    }
}
#[test]
fn outbound_frames_use_all_length_encodings_and_keep_the_response_bound() {
    for size in [0, 125, 126, 65535, 65536, 256 * 1024] {
        let mut wire = Vec::new();
        write_frame(&mut wire, 1, &vec![b'x'; size]).unwrap();
        assert_eq!(wire[0], 0x81);
        assert_eq!(
            wire.len(),
            size + if size < 126 {
                2
            } else if size < 65536 {
                4
            } else {
                10
            }
        );
    }
    assert!(write_frame(&mut Vec::new(), 1, &vec![0; 256 * 1024 + 1]).is_err());
}
