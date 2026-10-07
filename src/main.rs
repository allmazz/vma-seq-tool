mod utils;
mod vma;

use crate::utils::SequentialReader;
use crate::vma::{EXTENT_MAX_SIZE, VMAIndex, write_sequential};
use clap::{Args, Parser, Subcommand};
use std::fs::File;
use std::io;
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Args)]
struct IOArgs {
    /// Input VMA file. Use "-" to read from stdin (supported only during indexing).
    #[arg(value_name = "input")]
    input: PathBuf,

    /// Output file. Use "-" to output to stdout.
    #[arg(value_name = "output")]
    output: PathBuf,
}

impl IOArgs {
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.input.as_os_str().is_empty(), "Input is not specified");
        anyhow::ensure!(!self.output.as_os_str().is_empty(), "Output is not specified");
        anyhow::ensure!(
            self.output != self.input || self.is_stdout(),
            "Input can't be the same as output"
        );
        Ok(())
    }

    pub fn is_stdin(&self) -> bool {
        self.input.as_os_str() == "-"
    }

    pub fn is_stdout(&self) -> bool {
        self.output.as_os_str() == "-"
    }
}

#[derive(Subcommand)]
enum Command {
    Rebuild {
        #[command(flatten)]
        io: IOArgs,

        #[arg(short = 'u', long, value_name = "UUID", verbatim_doc_comment)]
        /// Rewrite the UUID stored in the headers.
        /// Reuse a stable UUID across backups to improve deduplication.
        /// Example: aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee
        rewrite_uuid: Option<Uuid>,

        #[arg(short = 'i', long, value_name = "index file", verbatim_doc_comment)]
        /// Load an index from a file or stdin ("-"). If not specified, indexing is performed before rebuilding.
        /// WARNING: A stale or corrupted index will silently corrupt the output, even if its saved header matches.
        index_input: Option<PathBuf>,
    },
    Index {
        #[command(flatten)]
        io: IOArgs,
    },
}

#[derive(Parser)]
#[command(version, about = "Makes Proxmox backups deduplication-friendly")]
struct CliArgs {
    #[arg(short = 's', long, verbatim_doc_comment)]
    /// Read the file sequentially in buffered chunks during indexing instead of seeking.
    /// By default, the indexer seeks to each extent header. This is fast on SSDs but may
    /// be slow on HDDs. Ignored when reading from stdin.
    sequential_scan: bool,

    #[arg(value_name = "bytes", long, default_value = "8388608", verbatim_doc_comment)]
    /// Size of the buffer in bytes when using sequential scan or reading from stdin. Minimum: 3867136.
    sequential_buffer_size: usize,

    #[arg(short = 'p', long, verbatim_doc_comment)]
    /// Show a progress bar.
    progress_bar: bool,

    #[arg(long, verbatim_doc_comment)]
    /// Ignore header and device size limits. Use only if you are certain the input file is not corrupted and you have enough RAM.
    ignore_limits: bool,

    #[command(subcommand)]
    command: Command,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = CliArgs::parse();
    if args.sequential_buffer_size < EXTENT_MAX_SIZE {
        anyhow::bail!("Sequential buffer must be at least {} bytes in size", EXTENT_MAX_SIZE);
    }

    match &args.command {
        Command::Rebuild {
            io,
            rewrite_uuid,
            index_input,
        } => {
            io.check()?;
            anyhow::ensure!(
                !io.is_stdin(),
                "Input can't be stdin for rebuild because random I/O is necessary"
            );
            rebuild(&args, io, rewrite_uuid.as_ref(), index_input.as_ref())
        }
        Command::Index { io } => {
            io.check()?;
            let index = index(&args, &io)?;
            if io.is_stdout() {
                let mut w = BufWriter::new(io::stdout().lock());
                index.save(&mut w)?;
                w.flush()?;
            } else {
                let mut w = BufWriter::new(File::create_new(&io.output)?);
                index.save(&mut w)?;
                log::info!("Index saved to {}", io.output.display());
                w.flush()?;
            }
            Ok(())
        }
    }
}

fn rebuild(
    args: &CliArgs,
    io: &IOArgs,
    rewrite_uuid: Option<&Uuid>,
    index_input: Option<&PathBuf>,
) -> anyhow::Result<()> {
    let mut input = File::open(&io.input)?;

    let build_index = index_input.is_none();
    let mut idx = if args.sequential_scan {
        let mut r = SequentialReader::new(&mut input, args.sequential_buffer_size)?;
        VMAIndex::from_sequential(&mut r, args.progress_bar, build_index, args.ignore_limits)?
    } else {
        VMAIndex::from_file(&mut input, args.progress_bar, build_index, args.ignore_limits)?
    };

    if !build_index {
        let index_input = index_input.unwrap();
        anyhow::ensure!(
            index_input != &io.input && (index_input != &io.output || io.is_stdout()),
            "Index input can't be the same as input or output"
        );
        if index_input.as_os_str() == "-" {
            let mut w = BufReader::new(io::stdin().lock());
            idx.load_saved(&mut w)?
        } else {
            let mut w = BufReader::new(File::open(index_input)?);
            idx.load_saved(&mut w)?
        }
    }

    if io.is_stdout() {
        let mut stdout = io::stdout().lock();
        write_sequential(&mut input, &mut stdout, &mut idx, rewrite_uuid, args.progress_bar)?;
        stdout.flush()?;
        return Ok(());
    }

    let mut file = File::create_new(&io.output)?;
    write_sequential(&mut input, &mut file, &mut idx, rewrite_uuid, args.progress_bar)?;
    file.flush()?;
    Ok(())
}

fn index(args: &CliArgs, io: &IOArgs) -> anyhow::Result<VMAIndex> {
    if io.is_stdin() {
        let mut stdin = io::stdin().lock();
        let mut r = SequentialReader::new(&mut stdin, args.sequential_buffer_size)?;
        return VMAIndex::from_sequential(&mut r, args.progress_bar, true, args.ignore_limits);
    }

    let mut input = File::open(&io.input)?;
    if args.sequential_scan {
        let mut r = SequentialReader::new(&mut input, args.sequential_buffer_size)?;
        return VMAIndex::from_sequential(&mut r, args.progress_bar, true, args.ignore_limits);
    }
    VMAIndex::from_file(&mut input, args.progress_bar, true, args.ignore_limits)
}
