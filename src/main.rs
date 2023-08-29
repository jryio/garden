#[macro_use]
extern crate lazy_static;

use clap::Parser;
use craft_files::CraftDocs;
use slug::slugify;
use std::path::{Path, PathBuf};

use crate::zola_files::ZolaFiles;

mod craft_files;
mod zola_files;

/// C2Z is a simple program to parse Craft exported Markdown files and convert them into Zola
/// compatible markdown files
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Input directory is a path to Craft's exported markdown directory
    ///
    /// This directory's name will be used when created a sub directory
    /// inside Zola's /content dir
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory is a path to the Zola `content/` directory
    ///
    /// TODO: What to do if there is already a directory present which matches the input
    /// directory's name? Write over? Probably.
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let input_dir = cli.input;
    let output_dir = cli.output;
    let mut zola = ZolaFiles::new(output_dir);
    let mut craft = CraftDocs::new(input_dir);
    craft.process_files()?;
    craft.format_markdown()?;
    zola.write_files(craft)?;

    // fs::create_dir_all("/Users/CASE/Downloads/my-new-directory/nested-one/nested-two")?;
    // fs::write(
    //     "/Users/CASE/Downloads/my-new-directory/hello.txt",
    //     "This was inserted into a new path",
    // )?;
    // fs::write(
    //     "/Users/CASE/Downloads/my-new-directory/nested-one/nested-two/hello.txt",
    //     "This was inserted into a new path",
    // )?;
    // fs::create_dir_all("/Users/CASE/Downloads/my-new-directory")?;

    Ok(())
}
