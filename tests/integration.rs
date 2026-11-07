use rstest::rstest;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;
use uuid::Uuid;

const BIN: &str = env!("CARGO_BIN_EXE_vma-seq-tool");
const UUID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const FLAGS: &[&[&str]] = &[&[], &["-s"], &["-p"], &["-p", "-s"]];

fn workspace(prefix: &str) -> TempDir {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(root).unwrap();
    tempfile::Builder::new().prefix(prefix).tempdir_in(root).unwrap()
}

fn tool(dir: &Path, flags: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command.current_dir(dir).args(flags).stdin(Stdio::null());
    command
}

fn run(command: &mut Command) -> Output {
    command.output().unwrap_or_else(|error| panic!("{command:?}: {error}"))
}

fn check(command: &mut Command) {
    let output = run(command);
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_file(command: &mut Command, expected: &Path, actual: &Path) {
    check(command);
    assert!(
        file_diff::diff_files(&mut File::open(expected).unwrap(), &mut File::open(actual).unwrap()),
        "{command:?}: {} differs from {}",
        actual.display(),
        expected.display()
    );
}

fn extract(dir: &Path, archive: &Path, destination: &str) {
    check(Command::new("vma").arg("list").arg(archive));
    check(Command::new("vma").args(["verify", "-v"]).arg(archive));
    check(
        Command::new("vma")
            .arg("extract")
            .arg(archive)
            .arg(dir.join(destination)),
    );
}

fn archive_uuid(path: &Path) -> Uuid {
    let mut header = [0; 24];
    File::open(path).unwrap().read_exact(&mut header).unwrap();
    Uuid::from_slice(&header[8..]).unwrap()
}

#[rstest]
fn index_matches_for_all_io_modes(#[files("tests/samples/*.vma")] sample: PathBuf) {
    let sample = sample.canonicalize().unwrap();
    let temp = workspace(sample.file_name().unwrap().to_str().unwrap());
    let dir = temp.path();
    check(tool(dir, &[]).arg("index").arg(&sample).arg("index"));

    for flags in FLAGS {
        for input in [sample.as_path(), Path::new("-")] {
            for output in ["out", "-"] {
                let mut command = tool(dir, flags);
                command.arg("index").arg(input).arg(output);
                if input == Path::new("-") {
                    command.stdin(File::open(&sample).unwrap());
                }
                if output == "-" {
                    command.stdout(File::create(dir.join("out")).unwrap());
                }
                check_file(&mut command, &dir.join("index"), &dir.join("out"));
                fs::remove_file(dir.join("out")).unwrap();
            }
        }
    }
}

#[rstest]
fn rebuild_preserves_archive_contents(#[files("tests/samples/*.vma")] sample: PathBuf) {
    let sample = sample.canonicalize().unwrap();
    let temp = workspace(sample.file_name().unwrap().to_str().unwrap());
    let dir = temp.path();
    extract(dir, &sample, "original");
    check(tool(dir, &[]).arg("index").arg(&sample).arg("index"));

    for rewrite_uuid in [None, Some(UUID)] {
        let mut command = tool(dir, &[]);
        command.arg("rebuild");
        if let Some(uuid) = rewrite_uuid {
            command.args(["-u", uuid]);
        }
        check(command.arg(&sample).arg("rebuilt"));
        extract(dir, &dir.join("rebuilt"), "extracted");
        let diff = dircmp::Comparison::default()
            .compare(dir.join("original"), dir.join("extracted"))
            .unwrap();
        assert!(diff.is_empty(), "{command:?}: {diff:?}");
        let expected_uuid = rewrite_uuid.map_or_else(|| archive_uuid(&sample), |uuid| uuid.parse().unwrap());
        assert_eq!(archive_uuid(&dir.join("rebuilt")), expected_uuid, "{command:?}");

        for flags in FLAGS {
            for index in [None, Some("index"), Some("-")] {
                for output in ["out", "-"] {
                    let mut command = tool(dir, flags);
                    command.arg("rebuild");
                    if let Some(uuid) = rewrite_uuid {
                        command.args(["-u", uuid]);
                    }
                    if let Some(index) = index {
                        command.args(["-i", index]);
                        if index == "-" {
                            command.stdin(File::open(dir.join("index")).unwrap());
                        }
                    }
                    command.arg(&sample).arg(output);
                    if output == "-" {
                        command.stdout(File::create(dir.join("out")).unwrap());
                    }
                    check_file(&mut command, &dir.join("rebuilt"), &dir.join("out"));
                    fs::remove_file(dir.join("out")).unwrap();
                }
            }
        }
        fs::remove_file(dir.join("rebuilt")).unwrap();
        fs::remove_dir_all(dir.join("extracted")).unwrap();
    }
}

#[rstest]
#[case::empty(0)]
#[case::header_without_blob(12288)]
#[case::truncated_blob(12500)]
fn rejects_truncated_input(#[case] len: usize) {
    let temp = workspace("truncated-input-");
    let dir = temp.path();
    let sample = fs::read("tests/samples/pve9_2_26-one-disk.vma").unwrap();
    fs::write(dir.join("truncated.vma"), &sample[..len]).unwrap();

    for flags in FLAGS {
        for input in ["truncated.vma", "-"] {
            let mut command = tool(dir, flags);
            command.args(["index", input, "out"]);
            if input == "-" {
                command.stdin(File::open(dir.join("truncated.vma")).unwrap());
            }
            let output = run(&mut command);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{command:?}: {stderr}");
            assert!(!dir.join("out").exists(), "{command:?}: created output on failure");
        }
    }
}

#[test]
fn accepts_out_of_range_ctime() {
    let temp = workspace("out-of-range-ctime-");
    let dir = temp.path();
    let mut sample = fs::read("tests/samples/pve9_2_26-one-disk.vma").unwrap();
    let header_size = u32::from_be_bytes(sample[56..60].try_into().unwrap()) as usize;
    sample[24..32].copy_from_slice(&i64::MAX.to_be_bytes());
    sample[32..48].fill(0);
    let digest = md5::compute(&sample[..header_size]);
    sample[32..48].copy_from_slice(digest.as_ref());
    fs::write(dir.join("input.vma"), sample).unwrap();

    check(
        tool(dir, &[])
            .env("RUST_LOG", "info")
            .args(["index", "input.vma", "out"]),
    );
}

#[rstest]
#[case::stdin(&["rebuild", "-", "out"], 1, "Input can't be stdin for rebuild")]
#[case::stdin_with_index_file(&["rebuild", "-i", "index", "-", "out"], 1, "Input can't be stdin for rebuild")]
#[case::stdin_with_index_stdin(&["rebuild", "-i", "-", "-", "out"], 1, "Input can't be stdin for rebuild")]
#[case::invalid_uuid(&["rebuild", "-u", "invalid-uuid", "unused.vma", "out"], 2, "invalid value 'invalid-uuid'")]
fn rejects_invalid_arguments(#[case] args: &[&str], #[case] status: i32, #[case] message: &str) {
    let temp = workspace("invalid-arguments-");
    let dir = temp.path();
    for flags in FLAGS {
        let mut command = tool(dir, flags);
        command.args(args);
        let output = run(&mut command);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(status), "{command:?}: {stderr}");
        assert!(stderr.contains(message), "{command:?}: {stderr}");
        assert!(!dir.join("out").exists(), "{command:?}: created output on failure");
    }
}
