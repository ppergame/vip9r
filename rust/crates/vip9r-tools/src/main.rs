use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use md5::{Digest, Md5};
use vip9r_core::{
    DecodeError, DecodeOutcome, Decoder, FrameInfo, I420Frame, OwnedWorkspace, Plane, PlaneShape,
    WorkspaceLayout, split_packet,
};

const DEFAULT_MEDIA_ROOT_ENV: &str = "VIP9R_MEDIA_ROOT";
const DEFAULT_MEDIA_ROOT: &str = "/bulk/vip9r";
const DEFAULT_BEAR_IVF: &str = "chromium/bear-vp9.ivf";

fn main() -> Result<()> {
    match Command::parse(env::args())? {
        Some(Command::Golden(args)) => run_golden(args),
        None => Ok(()),
    }
}

fn run_golden(args: GoldenArgs) -> Result<()> {
    let report = compare_ivf_to_golden(&args.input, &args.golden)?;
    report.print();

    if !report.passes(args.allow_mismatch) {
        bail!(
            "golden mismatch: {} matched, {} mismatched, {} missing, {} extra",
            report.matched_count(),
            report.mismatch_count(),
            report.missing_count(),
            report.extra_count()
        );
    }

    Ok(())
}

enum Command {
    Golden(GoldenArgs),
}

impl Command {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut args = args.into_iter();
        let _program = args.next();
        let Some(command) = args.next() else {
            print_usage();
            return Ok(None);
        };

        match command.as_str() {
            "-h" | "--help" => {
                print_usage();
                Ok(None)
            }
            "golden" => GoldenArgs::parse(args).map(|args| args.map(Self::Golden)),
            _ if command.starts_with('-') => bail!("unknown option: {command}"),
            _ => bail!("unknown command: {command}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GoldenArgs {
    input: PathBuf,
    golden: PathBuf,
    allow_mismatch: bool,
}

impl GoldenArgs {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut allow_mismatch = false;
        let mut paths = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-h" | "--help" => {
                    print_golden_usage();
                    return Ok(None);
                }
                "--allow-mismatch" => allow_mismatch = true,
                _ if arg.starts_with('-') => bail!("unknown argument: {arg}"),
                _ => paths.push(PathBuf::from(arg)),
            }
        }

        if paths.len() > 2 {
            bail!("usage: too many paths");
        }

        let input = paths.first().cloned().unwrap_or_else(default_bear_ivf_path);
        let golden = paths.get(1).cloned().unwrap_or_else(|| md5_sidecar(&input));

        Ok(Some(Self {
            input,
            golden,
            allow_mismatch,
        }))
    }
}

fn print_usage() {
    println!(
        "usage: vip9r-tools <command> [args]

Commands:
  golden    compare native decoder output to libvpx md5 sidecars

Run `vip9r-tools <command> --help` for command-specific help."
    );
}

fn print_golden_usage() {
    println!(
        "usage: vip9r-tools golden [--allow-mismatch] [input.ivf [input.ivf.md5]]

Compares decoded shown frames to libvpx md5 sidecars.
With no paths, uses ${DEFAULT_MEDIA_ROOT_ENV}/{DEFAULT_BEAR_IVF}, or {DEFAULT_MEDIA_ROOT}/{DEFAULT_BEAR_IVF}.
Set --allow-mismatch for M1 smoke runs where wrong frame contents are expected."
    );
}

fn default_bear_ivf_path() -> PathBuf {
    env::var_os(DEFAULT_MEDIA_ROOT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MEDIA_ROOT))
        .join(DEFAULT_BEAR_IVF)
}

fn md5_sidecar(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.md5",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

fn compare_ivf_to_golden(input_path: &Path, golden_path: &Path) -> Result<ComparisonReport> {
    let input = fs::read(input_path).with_context(|| format!("read {}", input_path.display()))?;
    let ivf = IvfFile::parse(&input).with_context(|| format!("parse {}", input_path.display()))?;
    if &ivf.header.fourcc != b"VP90" {
        bail!(
            "unsupported IVF fourcc: {}",
            String::from_utf8_lossy(&ivf.header.fourcc)
        );
    }

    let golden_text = fs::read_to_string(golden_path)
        .with_context(|| format!("read {}", golden_path.display()))?;
    let golden =
        parse_golden(&golden_text).with_context(|| format!("parse {}", golden_path.display()))?;

    let layout = WorkspaceLayout::new(ivf.header.width.into(), ivf.header.height.into())
        .map_err(|err| anyhow!("workspace layout: {err:?}"))?;
    let mut decoder = Decoder::new(layout);
    let mut workspace =
        OwnedWorkspace::new(layout).map_err(|err| anyhow!("create workspace: {err:?}"))?;

    let mut sink = Md5Sink::new(&golden);
    let mut decoded_coded_frames = 0u64;

    for packet in &ivf.frames {
        let coded_frames = split_packet(packet.payload)
            .map_err(|err| decode_packet_error(packet.index, packet.timestamp, err))?;
        decoded_coded_frames += coded_frames.len() as u64;

        for (coded_index, range) in coded_frames.as_slice().iter().copied().enumerate() {
            let coded_frame = range
                .as_slice(packet.payload)
                .map_err(|err| decode_packet_error(packet.index, packet.timestamp, err))?;
            let mut workspace_view = workspace.as_workspace();
            match decoder
                .decode_coded_frame(coded_frame, &mut workspace_view)
                .map_err(|err| decode_coded_frame_error(packet.index, coded_index, err))?
            {
                DecodeOutcome::NoOutput => {}
                DecodeOutcome::Output(frame) => sink.frame(frame)?,
            }
        }
    }

    let comparisons = sink.into_comparisons();

    Ok(ComparisonReport {
        input_path: input_path.to_owned(),
        golden_path: golden_path.to_owned(),
        ivf_header: ivf.header,
        ivf_packet_count: ivf.frames.len(),
        decoded_coded_frames,
        reported_shown_frames: comparisons.len() as u64,
        comparisons,
        expected_count: golden.len(),
    })
}

fn decode_packet_error(index: usize, timestamp: u64, err: DecodeError) -> anyhow::Error {
    anyhow!("decode packet {index} timestamp {timestamp}: {err:?}")
}

fn decode_coded_frame_error(
    packet_index: usize,
    coded_index: usize,
    err: DecodeError,
) -> anyhow::Error {
    anyhow!("decode packet {packet_index} coded frame {coded_index}: {err:?}")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IvfHeader {
    fourcc: [u8; 4],
    width: u16,
    height: u16,
    timebase_denominator: u32,
    timebase_numerator: u32,
    declared_frame_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IvfFrame<'a> {
    index: usize,
    timestamp: u64,
    payload: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IvfFile<'a> {
    header: IvfHeader,
    frames: Vec<IvfFrame<'a>>,
}

impl<'a> IvfFile<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 32 {
            bail!("IVF header is truncated");
        }
        if &data[0..4] != b"DKIF" {
            bail!("IVF signature is not DKIF");
        }

        let version = read_u16(data, 4)?;
        if version != 0 {
            bail!("unsupported IVF version: {version}");
        }

        let header_len = usize::from(read_u16(data, 6)?);
        if header_len < 32 {
            bail!("IVF header length is too small: {header_len}");
        }
        if data.len() < header_len {
            bail!("IVF header length exceeds file size: {header_len}");
        }

        let mut fourcc = [0; 4];
        fourcc.copy_from_slice(&data[8..12]);
        let width = read_u16(data, 12)?;
        let height = read_u16(data, 14)?;
        if width == 0 || height == 0 {
            bail!("IVF dimensions must be non-zero: {width}x{height}");
        }

        let header = IvfHeader {
            fourcc,
            width,
            height,
            timebase_denominator: read_u32(data, 16)?,
            timebase_numerator: read_u32(data, 20)?,
            declared_frame_count: read_u32(data, 24)?,
        };

        let mut frames = Vec::new();
        let mut offset = header_len;
        while offset < data.len() {
            let index = frames.len();
            if data.len() - offset < 12 {
                bail!("packet {index} header is truncated");
            }

            let frame_size = usize::try_from(read_u32(data, offset)?)
                .context("IVF packet size does not fit usize")?;
            let timestamp = read_u64(data, offset + 4)?;
            let payload_start = offset + 12;
            let payload_end = payload_start
                .checked_add(frame_size)
                .context("IVF packet size overflow")?;
            if payload_end > data.len() {
                bail!("packet {index} payload is truncated");
            }

            frames.push(IvfFrame {
                index,
                timestamp,
                payload: &data[payload_start..payload_end],
            });
            offset = payload_end;
        }

        if frames.is_empty() {
            bail!("IVF contains no packets");
        }

        Ok(Self { header, frames })
    }
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = read_array::<2>(data, offset)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = read_array::<4>(data, offset)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64> {
    let bytes = read_array::<8>(data, offset)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_array<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N]> {
    let end = offset.checked_add(N).context("offset overflow")?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| anyhow!("read past end at offset {offset}"))?;
    Ok(bytes.try_into().expect("slice length is fixed"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GoldenFrame {
    md5: String,
    name: String,
}

fn parse_golden(text: &str) -> Result<Vec<GoldenFrame>> {
    let mut frames = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        if line.trim().is_empty() {
            continue;
        }

        let mut fields = line.split_whitespace();
        let md5 = fields
            .next()
            .ok_or_else(|| anyhow!("line {line_number}: missing md5"))?;
        let name = fields
            .next()
            .ok_or_else(|| anyhow!("line {line_number}: missing frame name"))?;
        if fields.next().is_some() {
            bail!("line {line_number}: too many fields");
        }
        if !is_md5_hex(md5) {
            bail!("line {line_number}: invalid md5: {md5}");
        }

        frames.push(GoldenFrame {
            md5: md5.to_ascii_lowercase(),
            name: name.to_owned(),
        });
    }

    if frames.is_empty() {
        bail!("golden contains no frames");
    }

    Ok(frames)
}

fn is_md5_hex(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameComparison {
    frame_number: usize,
    expected_md5: Option<String>,
    expected_name: Option<String>,
    actual_md5: String,
    info: FrameInfo,
}

impl FrameComparison {
    fn is_match(&self) -> bool {
        self.expected_md5.as_deref() == Some(self.actual_md5.as_str())
    }
}

struct Md5Sink<'a> {
    expected: &'a [GoldenFrame],
    scratch: Vec<u8>,
    comparisons: Vec<FrameComparison>,
}

impl<'a> Md5Sink<'a> {
    fn new(expected: &'a [GoldenFrame]) -> Self {
        Self {
            expected,
            scratch: Vec::new(),
            comparisons: Vec::new(),
        }
    }

    fn into_comparisons(self) -> Vec<FrameComparison> {
        self.comparisons
    }
}

impl Md5Sink<'_> {
    fn frame(&mut self, frame: I420Frame<'_>) -> Result<()> {
        let byte_len = frame
            .info
            .i420_len()
            .ok_or_else(|| anyhow!("invalid frame dimensions: {:?}", frame.info))?;
        self.scratch.resize(byte_len, 0);
        let written = write_compact_i420(frame, &mut self.scratch)
            .map_err(|err| anyhow!("write compact I420: {err:?}"))?;

        let actual_md5 = md5_hex(&self.scratch[..written]);
        let expected = self.expected.get(self.comparisons.len());

        self.comparisons.push(FrameComparison {
            frame_number: self.comparisons.len() + 1,
            expected_md5: expected.map(|frame| frame.md5.clone()),
            expected_name: expected.map(|frame| frame.name.clone()),
            actual_md5,
            info: frame.info,
        });

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompactI420Error {
    OutputTooSmall { required: usize },
    InvalidPlane,
}

fn write_compact_i420(frame: I420Frame<'_>, output: &mut [u8]) -> Result<usize, CompactI420Error> {
    let byte_len = frame
        .info
        .i420_len()
        .ok_or(CompactI420Error::InvalidPlane)?;
    if output.len() < byte_len {
        return Err(CompactI420Error::OutputTooSmall { required: byte_len });
    }
    validate_compact_i420_layout(frame)?;

    let mut written = 0;
    written += copy_compact_i420_plane(&mut output[written..], frame.y)?;
    written += copy_compact_i420_plane(&mut output[written..], frame.u)?;
    written += copy_compact_i420_plane(&mut output[written..], frame.v)?;
    Ok(written)
}

fn validate_compact_i420_layout(frame: I420Frame<'_>) -> Result<(), CompactI420Error> {
    let chroma_width = frame.info.visible_width / 2 + frame.info.visible_width % 2;
    let chroma_height = frame.info.visible_height / 2 + frame.info.visible_height % 2;
    if frame.y.shape.width != frame.info.visible_width
        || frame.y.shape.height != frame.info.visible_height
        || frame.u.shape.width != chroma_width
        || frame.u.shape.height != chroma_height
        || frame.v.shape.width != chroma_width
        || frame.v.shape.height != chroma_height
    {
        return Err(CompactI420Error::InvalidPlane);
    }
    Ok(())
}

fn copy_compact_i420_plane(output: &mut [u8], plane: Plane<'_>) -> Result<usize, CompactI420Error> {
    let width = usize::try_from(plane.shape.width).map_err(|_| CompactI420Error::InvalidPlane)?;
    let height = usize::try_from(plane.shape.height).map_err(|_| CompactI420Error::InvalidPlane)?;
    if width == 0 || height == 0 || plane.shape.stride < width {
        return Err(CompactI420Error::InvalidPlane);
    }

    let last_row = plane
        .shape
        .stride
        .checked_mul(height - 1)
        .ok_or(CompactI420Error::InvalidPlane)?;
    let required_input = last_row
        .checked_add(width)
        .ok_or(CompactI420Error::InvalidPlane)?;
    if plane.data.len() < required_input {
        return Err(CompactI420Error::InvalidPlane);
    }

    let required_output = width
        .checked_mul(height)
        .ok_or(CompactI420Error::InvalidPlane)?;
    if output.len() < required_output {
        return Err(CompactI420Error::OutputTooSmall {
            required: required_output,
        });
    }

    for row in 0..height {
        let input_start = plane
            .shape
            .stride
            .checked_mul(row)
            .ok_or(CompactI420Error::InvalidPlane)?;
        let input_end = input_start + width;
        let output_start = width
            .checked_mul(row)
            .ok_or(CompactI420Error::InvalidPlane)?;
        let output_end = output_start + width;
        output[output_start..output_end].copy_from_slice(&plane.data[input_start..input_end]);
    }

    Ok(required_output)
}

fn md5_hex(input: &[u8]) -> String {
    let digest = Md5::digest(input);
    let digest_bytes: &[u8] = digest.as_ref();
    let mut output = String::with_capacity(32);
    for byte in digest_bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComparisonReport {
    input_path: PathBuf,
    golden_path: PathBuf,
    ivf_header: IvfHeader,
    ivf_packet_count: usize,
    decoded_coded_frames: u64,
    reported_shown_frames: u64,
    comparisons: Vec<FrameComparison>,
    expected_count: usize,
}

impl ComparisonReport {
    fn is_match(&self) -> bool {
        self.mismatch_count() == 0 && self.missing_count() == 0 && self.extra_count() == 0
    }

    fn passes(&self, allow_mismatch: bool) -> bool {
        self.is_match() || (allow_mismatch && self.missing_count() == 0 && self.extra_count() == 0)
    }

    fn matched_count(&self) -> usize {
        self.comparisons
            .iter()
            .filter(|comparison| comparison.is_match())
            .count()
    }

    fn mismatch_count(&self) -> usize {
        self.comparisons
            .iter()
            .filter(|comparison| comparison.expected_md5.is_some() && !comparison.is_match())
            .count()
    }

    fn missing_count(&self) -> usize {
        self.expected_count.saturating_sub(self.comparisons.len())
    }

    fn extra_count(&self) -> usize {
        self.comparisons.len().saturating_sub(self.expected_count)
    }

    fn print(&self) {
        let fourcc = String::from_utf8_lossy(&self.ivf_header.fourcc);
        println!("input: {}", self.input_path.display());
        println!("golden: {}", self.golden_path.display());
        println!(
            "ivf: fourcc={fourcc} size={}x{} timebase={}/{} declared_frames={} packets={}",
            self.ivf_header.width,
            self.ivf_header.height,
            self.ivf_header.timebase_numerator,
            self.ivf_header.timebase_denominator,
            self.ivf_header.declared_frame_count,
            self.ivf_packet_count
        );
        println!(
            "decoder: coded_frames={} shown_frames={}",
            self.decoded_coded_frames, self.reported_shown_frames
        );
        println!(
            "frames: {} matched, {} mismatched, {} missing, {} extra",
            self.matched_count(),
            self.mismatch_count(),
            self.missing_count(),
            self.extra_count()
        );

        for comparison in self
            .comparisons
            .iter()
            .filter(|comparison| !comparison.is_match())
            .take(10)
        {
            let expected = comparison.expected_md5.as_deref().unwrap_or("<none>");
            let name = comparison.expected_name.as_deref().unwrap_or("<extra>");
            println!(
                "mismatch frame {} {name}: expected {expected}, actual {}, size={}x{} render={}x{}",
                comparison.frame_number,
                comparison.actual_md5,
                comparison.info.visible_width,
                comparison.info.visible_height,
                comparison.info.render_width,
                comparison.info.render_height
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vip9r_core::FrameInfo;

    #[test]
    fn parses_ivf_header_and_packets() {
        let data = sample_ivf();
        let ivf = IvfFile::parse(&data).unwrap();

        assert_eq!(&ivf.header.fourcc, b"VP90");
        assert_eq!(ivf.header.width, 320);
        assert_eq!(ivf.header.height, 240);
        assert_eq!(ivf.header.timebase_denominator, 1000);
        assert_eq!(ivf.header.timebase_numerator, 1);
        assert_eq!(ivf.header.declared_frame_count, 2);
        assert_eq!(ivf.frames.len(), 2);
        assert_eq!(ivf.frames[0].timestamp, 0);
        assert_eq!(ivf.frames[0].payload, &[1, 2, 3]);
        assert_eq!(ivf.frames[1].timestamp, 1);
        assert_eq!(ivf.frames[1].payload, &[4, 5]);
    }

    #[test]
    fn rejects_truncated_ivf_packet_payload() {
        let mut data = sample_ivf();
        data.pop();

        let err = IvfFile::parse(&data).unwrap_err();

        assert!(err.to_string().contains("payload is truncated"));
    }

    #[test]
    fn parses_libvpx_md5_sidecar() {
        let frames = parse_golden(
            "4ff2537e44588e6473e236d8a6fc0054  img-320-240-0001.i420\n\
             8328efce9d9580304a3833a26a23321a  img-320-240-0002.i420\n",
        )
        .unwrap();

        assert_eq!(
            frames,
            vec![
                GoldenFrame {
                    md5: "4ff2537e44588e6473e236d8a6fc0054".to_owned(),
                    name: "img-320-240-0001.i420".to_owned()
                },
                GoldenFrame {
                    md5: "8328efce9d9580304a3833a26a23321a".to_owned(),
                    name: "img-320-240-0002.i420".to_owned()
                }
            ]
        );
    }

    #[test]
    fn md5_sink_hashes_compact_i420_bytes_in_luma_u_v_order() {
        let expected = vec![GoldenFrame {
            md5: md5_hex(&[1, 2, 3, 4, 5, 6]),
            name: "frame.i420".to_owned(),
        }];
        let mut sink = Md5Sink::new(&expected);

        sink.frame(tiny_i420_frame()).unwrap();

        let comparisons = sink.into_comparisons();
        assert_eq!(comparisons.len(), 1);
        assert!(comparisons[0].is_match());
        assert_eq!(
            comparisons[0].expected_md5.as_deref(),
            Some(expected[0].md5.as_str())
        );
    }

    #[test]
    fn compact_i420_write_strips_stride_and_orders_planes() {
        let info = FrameInfo::i420(3, 3, 3, 3, 0).unwrap();
        let y = [
            1, 2, 3, 99, //
            4, 5, 6, 99, //
            7, 8, 9, 99,
        ];
        let u = [
            10, 11, 99, //
            12, 13, 99,
        ];
        let v = [
            14, 15, 99, //
            16, 17, 99,
        ];
        let frame = I420Frame {
            info,
            y: plane(&y, 3, 3, 4),
            u: plane(&u, 2, 2, 3),
            v: plane(&v, 2, 2, 3),
        };

        let mut output = [0; 17];
        assert_eq!(write_compact_i420(frame, &mut output), Ok(17));
        assert_eq!(
            output,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]
        );
    }

    #[test]
    fn compact_i420_write_reports_whole_frame_len_when_output_is_too_small() {
        let frame = tiny_i420_frame();
        let mut output = [0; 5];

        assert_eq!(
            write_compact_i420(frame, &mut output),
            Err(CompactI420Error::OutputTooSmall { required: 6 })
        );
    }

    #[test]
    fn compact_i420_write_rejects_plane_dimension_mismatch() {
        let mut frame = tiny_i420_frame();
        frame.u.shape.width = 2;
        let mut output = [0; 6];

        assert_eq!(
            write_compact_i420(frame, &mut output),
            Err(CompactI420Error::InvalidPlane)
        );
    }

    #[test]
    fn args_default_to_bear_and_md5_sidecar() {
        let args = GoldenArgs::parse([]).unwrap().unwrap();

        assert!(args.input.ends_with("chromium/bear-vp9.ivf"));
        assert_eq!(
            args.golden.file_name().and_then(|name| name.to_str()),
            Some("bear-vp9.ivf.md5")
        );
        assert!(!args.allow_mismatch);
    }

    #[test]
    fn command_parser_routes_to_golden_subcommand() {
        let command = Command::parse([
            "vip9r-tools".to_owned(),
            "golden".to_owned(),
            "--allow-mismatch".to_owned(),
            "input.ivf".to_owned(),
        ])
        .unwrap()
        .unwrap();

        let Command::Golden(args) = command;
        assert!(args.allow_mismatch);
        assert_eq!(args.input, PathBuf::from("input.ivf"));
        assert_eq!(args.golden, PathBuf::from("input.ivf.md5"));
    }

    #[test]
    fn allow_mismatch_does_not_allow_missing_or_extra_frames() {
        let mut report = sample_report(vec![sample_comparison(Some("not-the-md5"))], 1);
        assert!(!report.passes(false));
        assert!(report.passes(true));

        report.expected_count = 2;
        assert!(!report.passes(true));

        report.expected_count = 0;
        assert!(!report.passes(true));
    }

    fn sample_report(comparisons: Vec<FrameComparison>, expected_count: usize) -> ComparisonReport {
        ComparisonReport {
            input_path: PathBuf::from("input.ivf"),
            golden_path: PathBuf::from("input.ivf.md5"),
            ivf_header: IvfHeader {
                fourcc: *b"VP90",
                width: 2,
                height: 2,
                timebase_denominator: 1,
                timebase_numerator: 1,
                declared_frame_count: 1,
            },
            ivf_packet_count: 1,
            decoded_coded_frames: 1,
            reported_shown_frames: comparisons.len() as u64,
            comparisons,
            expected_count,
        }
    }

    fn sample_comparison(expected_md5: Option<&str>) -> FrameComparison {
        FrameComparison {
            frame_number: 1,
            expected_md5: expected_md5.map(str::to_owned),
            expected_name: Some("frame.i420".to_owned()),
            actual_md5: "actual-md5".to_owned(),
            info: FrameInfo::i420(2, 2, 2, 2, 0).unwrap(),
        }
    }

    fn sample_ivf() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"DKIF");
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(b"VP90");
        data.extend_from_slice(&320u16.to_le_bytes());
        data.extend_from_slice(&240u16.to_le_bytes());
        data.extend_from_slice(&1000u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&[1, 2, 3]);
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&[4, 5]);
        data
    }

    fn tiny_i420_frame() -> I420Frame<'static> {
        I420Frame {
            info: FrameInfo::i420(2, 2, 2, 2, 0).unwrap(),
            y: plane(&[1, 2, 3, 4], 2, 2, 2),
            u: plane(&[5], 1, 1, 1),
            v: plane(&[6], 1, 1, 1),
        }
    }

    fn plane(data: &[u8], width: u32, height: u32, stride: usize) -> Plane<'_> {
        Plane {
            data,
            shape: PlaneShape::new(width, height, stride),
        }
    }
}
