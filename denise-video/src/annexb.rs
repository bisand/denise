//! Annex-B elementary streams, split into what a decoder wants fed.
//!
//! An `.h264`/`.h265` file is NAL units separated by start codes — three or
//! four bytes of `00 00 (00) 01`. A stateful V4L2 decoder wants **one access
//! unit per buffer**: every NAL belonging to one picture, prefix NALs (SPS,
//! PPS, SEI…) riding with the picture they precede. Feeding arbitrary chunks
//! works on some drivers and not others; feeding access units works on all of
//! them, so that is the one behaviour this module has.
//!
//! Everything here is pure — bytes in, byte ranges out — which is what makes
//! it the tested-and-mutated half of a crate whose other half needs a `/dev`
//! to talk to.

/// Which codec's NAL headers to read. The two entries of the format menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    /// H.264 / AVC: one-byte NAL header, `nal_unit_type` in bits 0..5.
    H264,
    /// H.265 / HEVC: two-byte NAL header, `nal_unit_type` in bits 1..7 of the
    /// first byte.
    H265,
}

/// One NAL unit inside a stream: where its payload starts (past the start
/// code) and ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Nal {
    /// First byte of the NAL header, past the start code.
    start: usize,
    /// One past the last payload byte.
    end: usize,
}

/// Splits a stream into NAL units by start code.
///
/// Both three- and four-byte start codes are accepted, mixed freely — encoders
/// emit both, often in the same stream. Bytes before the first start code are
/// garbage and skipped, which is what makes resuming mid-stream possible.
fn split_nals(stream: &[u8]) -> Vec<Nal> {
    let mut nals = Vec::new();
    let mut i = 0;
    let mut current: Option<usize> = None;
    while i + 2 < stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            // The start code may carry a leading zero; the previous NAL ends
            // before it either way.
            let code_start = if i > 0 && stream[i - 1] == 0 {
                i - 1
            } else {
                i
            };
            if let Some(start) = current.take() {
                nals.push(Nal {
                    start,
                    end: code_start,
                });
            }
            current = Some(i + 3);
            i += 3;
        } else if stream[i + 2] != 0 {
            // Cannot be inside a start code for two more positions.
            i += 3;
        } else {
            i += 1;
        }
    }
    // A trailing start code with no header byte after it is not a NAL: there
    // is nothing to classify, so there is nothing to emit.
    if let Some(start) = current
        && start < stream.len()
    {
        nals.push(Nal {
            start,
            end: stream.len(),
        });
    }
    nals
}

/// Whether a NAL is a VCL unit — a slice of an actual picture — for `codec`.
fn is_vcl(codec: Codec, header: u8) -> bool {
    match codec {
        // H.264: types 1..=5 are coded slices.
        Codec::H264 => matches!(header & 0x1F, 1..=5),
        // H.265: types 0..=31 are VCL.
        Codec::H265 => (header >> 1) & 0x3F <= 31,
    }
}

/// Whether a VCL NAL begins a new picture.
///
/// For H.264 the first slice of a picture has `first_mb_in_slice == 0`, which
/// is the first Exp-Golomb symbol after the header — a leading `1` bit encodes
/// zero, so testing the top bit of the first payload byte is exact. For H.265
/// the slice header starts with a plain `first_slice_segment_in_pic_flag` bit.
fn starts_picture(codec: Codec, payload: &[u8]) -> bool {
    match codec {
        Codec::H264 => payload.get(1).is_some_and(|&b| b & 0x80 != 0),
        Codec::H265 => payload.get(2).is_some_and(|&b| b & 0x80 != 0),
    }
}

/// An iterator of access units: for each, the byte range of the stream that
/// holds every NAL of one picture, prefixes included, start codes intact.
///
/// The ranges tile the stream in order (after any leading garbage), so a
/// caller can feed a decoder by slicing the mapped file — no copies here.
pub struct AccessUnits<'a> {
    stream: &'a [u8],
    nals: Vec<Nal>,
    /// Index of the first NAL not yet emitted.
    next: usize,
    codec: Codec,
}

impl<'a> AccessUnits<'a> {
    /// Prepares to walk `stream` as `codec`.
    pub fn new(stream: &'a [u8], codec: Codec) -> Self {
        Self {
            stream,
            nals: split_nals(stream),
            next: 0,
            codec,
        }
    }

    /// How many NAL units the stream holds — a sanity number for a probe.
    pub fn nal_count(&self) -> usize {
        self.nals.len()
    }

    /// The byte range of the start code introducing NAL `index`.
    fn unit_start(&self, index: usize) -> usize {
        let start = self.nals[index].start;
        // Walk back over the start code: 00 00 01, optionally preceded by 00.
        let code = start - 3;
        if code > 0 && self.stream[code - 1] == 0 {
            code - 1
        } else {
            code
        }
    }
}

impl<'a> Iterator for AccessUnits<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.nals.len() {
            return None;
        }
        let first = self.next;
        let mut seen_vcl = false;
        let mut end = self.nals.len();
        for i in first..self.nals.len() {
            let nal = self.nals[i];
            let header = self.stream[nal.start];
            let vcl = is_vcl(self.codec, header);
            if vcl {
                let payload = &self.stream[nal.start..nal.end];
                if seen_vcl && starts_picture(self.codec, payload) {
                    // A new picture begins; the unit ends before this NAL and
                    // before any prefix NALs that belong with it.
                    let mut boundary = i;
                    while boundary > first
                        && !is_vcl(self.codec, self.stream[self.nals[boundary - 1].start])
                    {
                        boundary -= 1;
                    }
                    // Prefixes can only be pulled forward if the unit keeps a
                    // VCL; the first unit of a stream is often pure prefix.
                    if (first..boundary)
                        .any(|j| is_vcl(self.codec, self.stream[self.nals[j].start]))
                    {
                        end = boundary;
                        break;
                    }
                }
                seen_vcl = true;
            }
        }
        self.next = end;
        let from = self.unit_start(first);
        let to = self.nals[end - 1].end;
        Some(&self.stream[from..to])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A NAL with a 4-byte start code. `header` is the NAL header byte;
    /// `first` sets the start-of-picture bit in the slice header.
    fn nal(header: u8, first: bool, len: usize) -> Vec<u8> {
        let mut out = vec![0, 0, 0, 1, header];
        match header & 0x1F {
            // H.264 slice: byte after the header carries first_mb_in_slice as
            // Exp-Golomb; a leading 1 bit is zero.
            1..=5 => out.push(if first { 0x80 } else { 0x40 }),
            _ => out.push(0xFF),
        }
        out.resize(out.len() + len, 0xAA);
        out
    }

    const SPS: u8 = 0x67; // type 7
    const PPS: u8 = 0x68; // type 8
    const IDR: u8 = 0x65; // type 5, VCL
    const SLICE: u8 = 0x41; // type 1, VCL

    #[test]
    fn nals_split_on_both_start_code_lengths() {
        let mut stream = vec![0, 0, 1, SPS, 0xFF, 0x11]; // 3-byte code
        stream.extend([0, 0, 0, 1, PPS, 0xFF, 0x22]); // 4-byte code
        let units = AccessUnits::new(&stream, Codec::H264);
        assert_eq!(units.nal_count(), 2);
    }

    #[test]
    fn garbage_before_the_first_start_code_is_skipped() {
        let mut stream = vec![0xDE, 0xAD, 0xBE];
        stream.extend(nal(SPS, false, 4));
        assert_eq!(AccessUnits::new(&stream, Codec::H264).nal_count(), 1);
    }

    #[test]
    fn an_access_unit_is_the_prefixes_plus_the_picture() {
        // SPS PPS IDR | SLICE(first) | SLICE(first): three pictures, the
        // first carrying its parameter sets.
        let mut stream = Vec::new();
        stream.extend(nal(SPS, false, 8));
        stream.extend(nal(PPS, false, 4));
        stream.extend(nal(IDR, true, 32));
        stream.extend(nal(SLICE, true, 16));
        stream.extend(nal(SLICE, true, 16));

        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H264).collect();
        assert_eq!(units.len(), 3, "three pictures, three units");
        assert!(units[0].len() > units[1].len(), "unit one carries SPS+PPS");
        // The units tile the stream exactly.
        let total: usize = units.iter().map(|u| u.len()).sum();
        assert_eq!(total, stream.len());
        assert_eq!(units[0], &stream[..units[0].len()]);
    }

    #[test]
    fn a_multi_slice_picture_stays_one_unit() {
        // IDR(first) SLICE(continuation) | SLICE(first): two pictures.
        let mut stream = Vec::new();
        stream.extend(nal(IDR, true, 16));
        stream.extend(nal(SLICE, false, 16)); // same picture, first_mb != 0
        stream.extend(nal(SLICE, true, 16));

        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H264).collect();
        assert_eq!(units.len(), 2, "a continuation slice must not split");
    }

    #[test]
    fn prefixes_between_pictures_ride_with_the_next_one() {
        // IDR(first) | SPS PPS SLICE(first): the parameter sets belong to the
        // picture after them, not the one before.
        let mut stream = Vec::new();
        stream.extend(nal(IDR, true, 16));
        stream.extend(nal(SPS, false, 8));
        stream.extend(nal(PPS, false, 4));
        stream.extend(nal(SLICE, true, 16));

        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H264).collect();
        assert_eq!(units.len(), 2);
        let idr_only = nal(IDR, true, 16);
        assert_eq!(units[0].len(), idr_only.len(), "unit one is the IDR alone");
        assert!(
            units[1].len() > nal(SLICE, true, 16).len(),
            "unit two took the prefixes"
        );
    }

    #[test]
    fn units_start_with_their_start_codes() {
        let mut stream = Vec::new();
        stream.extend(nal(IDR, true, 8));
        stream.extend(nal(SLICE, true, 8));
        for unit in AccessUnits::new(&stream, Codec::H264) {
            assert!(
                unit.starts_with(&[0, 0, 0, 1]) || unit.starts_with(&[0, 0, 1]),
                "a decoder is fed Annex-B, start codes included"
            );
        }
    }

    #[test]
    fn an_empty_or_garbage_stream_yields_nothing_and_nobody_panics() {
        assert_eq!(AccessUnits::new(&[], Codec::H264).count(), 0);
        assert_eq!(AccessUnits::new(&[0, 0], Codec::H264).count(), 0);
        assert_eq!(
            AccessUnits::new(&[0xFF; 64], Codec::H264).count(),
            0,
            "no start code, no units"
        );
        // A start code with nothing after it is not a NAL — there is no
        // header byte to classify.
        assert_eq!(AccessUnits::new(&[0, 0, 1], Codec::H264).count(), 0);
        // Truncated mid-slice-header: must not panic.
        let _ = AccessUnits::new(&[0, 0, 1, IDR], Codec::H264).count();
    }

    #[test]
    fn hevc_pictures_split_on_the_first_slice_flag() {
        // HEVC: two-byte NAL header. Type 19 (IDR_W_RADL) = header 0x26 0x01.
        // The bit after the header is first_slice_segment_in_pic_flag.
        let hevc_nal = |first: bool| -> Vec<u8> {
            let mut out = vec![0, 0, 0, 1, 0x26, 0x01];
            out.push(if first { 0x80 } else { 0x00 });
            out.resize(out.len() + 8, 0xAA);
            out
        };
        let mut stream = Vec::new();
        stream.extend(hevc_nal(true));
        stream.extend(hevc_nal(false)); // continuation
        stream.extend(hevc_nal(true));
        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H265).collect();
        assert_eq!(units.len(), 2);
    }

    #[test]
    fn payload_bytes_that_look_like_headers_do_not_split_units() {
        // A payload may contain 0x65 (an IDR header byte) — only bytes after a
        // real start code are headers. Encoders prevent 00 00 01 in payloads
        // via emulation prevention, so a payload byte alone must never split.
        let mut stream = nal(IDR, true, 0);
        stream.extend([0x65, 0x80, 0x41, 0x80, 0x67]); // header-lookalikes
        stream.extend(nal(SLICE, true, 4));
        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H264).collect();
        assert_eq!(units.len(), 2);
    }
}

#[cfg(test)]
mod real_stream {
    use super::*;

    /// Against a real encoder's output, not hand-built NALs. Ignored by
    /// default because the file is not in the tree:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i "testsrc2=size=640x360:rate=30:duration=6" \
    ///        -c:v libx264 -profile:v main -pix_fmt yuv420p -an -f h264 /tmp/testcard.h264
    /// DENISE_TEST_H264=/tmp/testcard.h264 cargo test -p denise-video -- --ignored
    /// ```
    #[test]
    #[ignore = "needs a stream on disk; see the doc comment"]
    fn a_real_encoder_stream_parses_to_its_frame_count() {
        let path = std::env::var("DENISE_TEST_H264").expect("DENISE_TEST_H264");
        let stream = std::fs::read(path).expect("read the stream");
        let units: Vec<&[u8]> = AccessUnits::new(&stream, Codec::H264).collect();
        // Six seconds at 30 fps: one access unit per frame, exactly.
        assert_eq!(units.len(), 180, "one access unit per encoded frame");
        // The units tile the stream completely — nothing dropped, nothing
        // double-counted — after any leading garbage (none from ffmpeg).
        let total: usize = units.iter().map(|u| u.len()).sum();
        assert_eq!(total, stream.len());
        for unit in &units {
            assert!(unit.starts_with(&[0, 0, 0, 1]) || unit.starts_with(&[0, 0, 1]));
        }
    }
}
