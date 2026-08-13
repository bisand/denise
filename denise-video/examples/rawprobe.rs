//! The probe with its skip reasons shown: every /dev/video* node, every step,
//! every errno. For boards where `probe` finds nothing it should.

#[cfg(target_os = "linux")]
fn main() {
    use denise_video::v4l2;
    use std::os::fd::AsFd;

    for n in 0..64u32 {
        let path = format!("/dev/video{n}");
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                println!("{path}: open failed: {e}");
                continue;
            }
        };
        let fd = file.as_fd();
        match v4l2::querycap(fd) {
            Err(e) => println!("{path}: QUERYCAP failed: {e}"),
            Ok(cap) => {
                println!(
                    "{path}: driver={} caps={:#010x} device_caps={:#010x}",
                    cap.driver_name(),
                    cap.capabilities,
                    cap.device_caps
                );
                for buf_type in [v4l2::BUF_TYPE_OUTPUT_MPLANE, v4l2::BUF_TYPE_CAPTURE_MPLANE] {
                    let side = if buf_type == v4l2::BUF_TYPE_OUTPUT_MPLANE {
                        "  output "
                    } else {
                        "  capture"
                    };
                    for index in 0..32 {
                        match v4l2::enum_fmt(fd, buf_type, index) {
                            Ok(Some(desc)) => {
                                let cc = desc.pixelformat.to_le_bytes();
                                println!(
                                    "{side} fmt[{index}]: {}{}{}{} flags={:#x}",
                                    cc[0] as char,
                                    cc[1] as char,
                                    cc[2] as char,
                                    cc[3] as char,
                                    desc.flags
                                );
                            }
                            Ok(None) => break,
                            Err(e) => {
                                println!("{side} ENUM_FMT[{index}] failed: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("V4L2 is Linux; run this on the board.");
}
