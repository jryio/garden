use anyhow::{bail, Context};
use regex::{Captures, Match, Regex};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::read_to_string,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use time::{macros::format_description, Date};
use unicode_segmentation::UnicodeSegmentation;

use slug::slugify;
use walkdir::{DirEntry, WalkDir};

lazy_static! {
    static ref RE_FIRST_H1: Regex = Regex::new(r"^\#(.*)\n").unwrap();
    // Names the capture group "link_name"
    //
    // I noticed that because I am using regular expressions on the note bodies, I accidentially
    // matched on some example code inside of a code-fence
    //
    // Example:
    // ```purescript
    // list = [[1,2,3], [4,5], [6]]
    // ```
    //
    // I realized that the wiki links regular expression should say
    // "match on all character inside two brackets, such that every character is itself not a bracket"
    //
    // However that does not exclude this code snippet
    // Example:
    // ```rust
    // [[ 1, 2, 3, 4, 5]]
    //
    // Basically if I come back to this program because it panicked for some reason, this may be a
    // reason why... yippie for Regexs!
    //```
    static ref RE_WIKI_LINK: Regex = Regex::new(r"\[\[(?<link_name>[^\[\]]+?)\]\]").unwrap();
    static ref RE_UUID_V4: Regex = Regex::new(
        r"\#\^[0-9A-Za-z]{8}-[0-9A-Za-z]{4}-4[0-9A-Za-z]{3}-[89ABab][0-9A-Za-z]{3}-[0-9A-Za-z]{12}"
    )
    .unwrap();
    // Names the capture group "header" and "link_name"
    static ref RE_HEADER_ANCHOR: Regex = Regex::new(r"(?<link_name>.+)(\#(?<header>.+))").unwrap();
    // Names the capture group "desc" for the date string,
    // "day_url" for everything including day://,
    // and "date" for the actual yyyy.mm.dd
    static ref RE_DAY_LINK: Regex =
        Regex::new(r"\[(?<desc>.*)\]\((?<day_url>day:\/\/(?<date>\d{4}\.\d{2}\.\d{2}))\)").unwrap();
    static ref RE_IMG_ASSET_LINK: Regex =
        Regex::new(r"\!\[(?<name>.*)?\]\((.*\.assets\/)(?<file_name>.*)\)").unwrap();
    static ref RE_CRAFTDOCS_LINK: Regex = Regex::new(r"\[.*\]\((craftdocs:\/\/open.*)\)").unwrap();
    static ref RE_CODE_BLOCK_OTHER: Regex =  Regex::new(r"```other").unwrap();
}

// =============

const ASSETS_DIR_EXT: &str = "assets";
const MD_EXT: &str = "md";
const BIN_EXT: &str = "bin";
const PNG_EXT: &str = "png";
const UNIC_EVERGREEN: char = '🌲';
const UNIC_POTTED: char = '🪴';
const UNIC_SEEDLING: char = '🌱';

// =============

fn strip_input_dir(p: &Path, input_dir: &PathBuf) -> anyhow::Result<PathBuf> {
    Ok(p.strip_prefix(input_dir)?.to_path_buf())
}
fn create_input_path(input_dir: &Path, p: &PathBuf) -> PathBuf {
    input_dir.join(p)
}

fn slugify_path(p: &Path) -> PathBuf {
    p.components()
        .map(|x| x.as_os_str().to_str().unwrap())
        .map(slugify)
        .collect::<PathBuf>()
}

// https://stackoverflow.com/a/76909909
fn strip_emoji(p: &Path) -> PathBuf {
    p.components()
        .map(|x| x.as_os_str().to_str().unwrap())
        .map(|x| {
            let graphemes = x.graphemes(true);
            let is_not_emoji = |x: &&str| emojis::get(x).is_none();
            graphemes.filter(is_not_emoji).collect::<String>()
        })
        .collect::<PathBuf>()
}

// =============

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum NoteType {
    Evergreen,
    Potted,
    Seedling,
    None,
}

impl NoteType {
    fn to_weight(&self) -> char {
        match self {
            Self::Evergreen => '1',
            Self::Potted => '2',
            Self::Seedling => '3',
            Self::None => '4',
        }
    }

    fn as_emoji(&self) -> char {
        match self {
            Self::Evergreen => UNIC_EVERGREEN,
            Self::Potted => UNIC_POTTED,
            Self::Seedling => UNIC_SEEDLING,
            Self::None => ' ',
        }
    }
}

impl Default for NoteType {
    fn default() -> Self {
        Self::None
    }
}

impl From<usize> for NoteType {
    fn from(value: usize) -> Self {
        // Lower values have high priority in Zola
        match value {
            0 => Self::Evergreen,
            1 => Self::Potted,
            2 => Self::Seedling,
            _ => Self::None,
        }
    }
}

impl From<&str> for NoteType {
    fn from(value: &str) -> Self {
        if value.contains(UNIC_EVERGREEN) {
            return Self::Evergreen;
        }
        if value.contains(UNIC_POTTED) {
            return Self::Potted;
        }
        if value.contains(UNIC_SEEDLING) {
            return Self::Seedling;
        }
        Self::default()
    }
}

#[derive(Default, Clone, Debug, Hash, PartialEq, Eq)]
pub struct FileData {
    /// Evergreen | Potted | Seedling | None
    pub note_type: NoteType,
    /// The original full filesystem path
    pub path_full: PathBuf,
    /// Path is the relative path to the INPUT directory without the file extension
    ///
    /// Example: `/Users/{user_name}/notes/` + `Cyptography/TLS`
    ///                                         ^_____________^____path
    pub path_rel: PathBuf,
    /// Slugified is the path_rel slugified on each component
    ///
    /// Example:
    ///     "🌲 Woodworking/Joinery Techniques/Dovetail Joint.md"
    ///     "woodworking/joinery-techniques/dovetail-join.md"
    pub path_slug: PathBuf,
    /// Name is the file name, unmodified, including any UTF-8 characters
    ///
    /// Example:
    ///     "{root_dir}/Expatriation/💰 The 30% Ruling.md"
    ///     "💰 The 30% Ruling"
    pub name: String,
    /// Assets is a Vec of asset paths which are used as links inside one of our markdown notes.
    ///
    /// Importantly, any asset will be co-located in the same directory so the markdown link will
    /// not need to have any paths, just the file name.
    ///
    /// This means that all entries in this vector will be relative to path_slug
    ///
    /// Example:
    /// path_slug = woodworking/joinery/index.md
    /// assets =
    pub assets: Option<Vec<PathBuf>>,
    ///  Asset Dir is the path to the containing directory of the markdown file's image assets
    ///  It is the relative path, including the ".assets" name on the directory
    ///
    ///  Example:
    ///  "Woodworking/Dovetail Joing.assets""
    pub assets_dir: Option<PathBuf>,
    // Contents is the file contents after we have processed it (replacements)
    pub contents: String,
    /// Craft will set this for us as its internal time of when the file was created
    pub created_at: String,
    /// Craft will set this for us as its internal time of when the file was modified
    pub modified_at: String,
}

impl FileData {
    pub fn set_paths(&mut self, input_dir: &PathBuf) -> anyhow::Result<()> {
        let mut path_rel = strip_input_dir(&self.path_full, input_dir)?;
        // Drop '.md' from the key, it is implied with files
        // Example : Woodworking/Joinery/Dovetail Joint
        path_rel.set_extension("");

        // Remove all emoji from the path_slug.
        // Otherwise they are convereted into their shortcode representation by slugify
        // Example:
        //  "🚀" -> "rocket" -> "aerospace-engineering/rocket-space-ship.md"
        //  "aerospace-engineering/space-ship.md"
        let mut path_slug = strip_emoji(&path_rel);

        // Slugify the path with no eomji hooray
        path_slug = slugify_path(&path_slug);

        // Add the extension back after slugification
        // This is because Zola wants a link in this format
        // [Page Name](@/garden/page-name.md)
        path_slug.set_extension(MD_EXT);

        self.path_slug = path_slug;
        self.path_rel = path_rel;
        Ok(())
    }
}

impl TryFrom<PathBuf> for FileData {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        let path_full = path.clone();
        // Drop extension (.md)
        let name = path.with_extension("");
        let name = name.file_name().with_context(|| {
            format!(
                "Received a path without a valid file name: path={}",
                path.display()
            )
        })?;
        let name = name
            .to_str()
            .context("File name failed to convert from OsStr to str")?;

        let note_type = NoteType::from(name);

        let metadata = path_full.metadata()?;
        let ctime: OffsetDateTime = metadata.created()?.into();
        let mtime: OffsetDateTime = metadata.modified()?.into();
        let created_at = ctime.format(&Rfc3339)?;
        let modified_at = mtime.format(&Rfc3339)?;

        Ok(Self {
            note_type,
            path_full,
            name: name.into(),
            path_rel: PathBuf::default(),
            path_slug: PathBuf::default(),
            assets: None,
            assets_dir: None,
            contents: String::default(),
            created_at,
            modified_at,
        })
    }
}

#[allow(dead_code)]
pub struct Directory {
    pub path_full: PathBuf,
    /// Based on the std::fs::rename function we should be renaming directories like so:
    /// https://doc.rust-lang.org/1.71.1/std/fs/fn.rename.html
    pub path_slug: PathBuf,
    pub name: String,
}

#[derive(Debug)]
pub struct CraftDocs {
    /// input_dir is the top level directory of the exported Craft fodler or space
    input_dir: PathBuf,
    /// The name of this directory will be used as the new name when imported into Zola
    input_dir_name: String,
    /// Directories is a unique set of paths to directories within the input_dir.
    ///
    /// This is used when parsing a file's markdown wiki style links to construct the final
    /// Zola internal markdown link
    ///
    /// Example
    /// * Craft Markdown File
    ///
    /// ```markdown
    /// This is beause [[Cryptography/TLS]] uses certificates
    /// ````
    ///
    ///
    /// * Zola Markdown File
    /// ```markdown
    /// This is because [TLS](@/{input_dir}/cryptography/tls/index.md) uses certificates
    /// ````
    pub directories: HashSet<PathBuf>,
    /// Files is a HashMap of file paths to file metadata
    ///
    /// Note: The key is the file's path WITHOUT the `.md` extension
    pub files: HashMap<PathBuf, FileData>,
}

impl CraftDocs {
    pub fn new(input_dir: PathBuf) -> Self {
        let input_dir_name = input_dir.file_name().unwrap_or_default();
        let input_dir_name: String = input_dir_name.to_str().unwrap_or_default().into();
        CraftDocs {
            input_dir,
            input_dir_name,
            directories: HashSet::new(),
            files: HashMap::new(),
        }
    }

    pub fn process_files(&mut self) -> anyhow::Result<()> {
        let files_first_cmp = |a: &DirEntry| if a.path().is_dir() { 2 } else { 0 };
        for entry in WalkDir::new(&self.input_dir).sort_by_key(files_first_cmp) {
            let entry = entry?;
            // Make path relative to the input dir
            let full_path = &entry.into_path();

            if full_path.is_dir() {
                if *full_path == self.input_dir {
                    continue;
                }
                self.set_directory(full_path.clone())?;
            }

            if full_path.is_file() {
                let file_name = full_path.file_name().with_context(|| {
                    format!("Unable to get file name for path = {}", full_path.display())
                })?;
                if file_name == ".DS_Store" {
                    continue;
                }
                self.set_file(full_path.clone())?;
            }
        }
        Ok(())
    }

    fn set_directory(&mut self, full_path: PathBuf) -> anyhow::Result<()> {
        let rel_path = strip_input_dir(&full_path, &self.input_dir)?;
        if let Some(ext) = full_path.extension() {
            if ext == ASSETS_DIR_EXT {
                // TODO: Skip setting this directory path on our FileData. Instead write a list of
                // file paths directly to a Vec<Path> on FileData files on our FileData struct.
                // Push them a Vec<PathBuf>
                self.set_asset_dir(rel_path)?;
            }
            return Ok(());
        }
        self.directories.insert(rel_path);
        Ok(())
    }

    fn set_file(&mut self, full_path: PathBuf) -> anyhow::Result<()> {
        let ext = full_path.extension()
            .with_context(||
                format!("Trying to create a FileData entry in HashMap but could not access the file's extension for file = {}", full_path.display())
            )?;

        // We have a file which is non-markdown, meaning it is an asset file. So we do not want to
        // use it as a key into our HashMap. Instead let set_asset_on_file lookup the corresponding
        // file in our HashTable to add this file as an associated asset path.
        if ext != MD_EXT {
            return self.set_asset_on_file(&full_path, ext);
        }

        let mut file_data = FileData::try_from(full_path.clone())?;
        // Set path_rel, path_slug
        file_data.set_paths(&self.input_dir)?;
        let key = file_data.path_rel.clone();
        // Insert into HashMap
        let _ = self.files.insert(key, file_data);
        Ok(())
    }

    fn set_asset_on_file(&mut self, asset_path: &Path, ext: &OsStr) -> anyhow::Result<()> {
        // No point in adding an ".bin" asset as there will also be a {name}_bin_preview.png in the
        // same directory
        if ext == BIN_EXT {
            return Ok(());
        }
        // The only {name}_{ext}_preview.png files we want to add to our FileData struct are the ones for
        // ".bin" files since only the preview can be used in Markdown.
        // All other {name}_{ext}_preview.png files generated by Craft are useless to us and will
        // not be copied over to Zola.
        //
        // NOTE: The file name includes the extension (e.g. "image.png")
        let fname = asset_path.file_name().with_context(|| {
            format!(
                "Attempted to get an asset file's file_name but failed. File path =  {}",
                asset_path.display()
            )
        })?;
        let fname = fname.to_str()
            .with_context(|| {
                format!(
                    "Attempted to convert an asset file's OsStr representation to UTF-8 str and failed. File path = {}",
                    asset_path.display()
                )
            })?;
        // Don't add files that are previews of non-bin assets (E.g. previews of jpegs or other pngs)
        let bin_preview = format!("_{BIN_EXT}_preview");
        if ext == PNG_EXT && !fname.contains(bin_preview.as_str()) {
            return Ok(());
        }

        // Set this asset on our FileData
        //
        // NOTE: We are assuming that the FileData for the original markdown file has already been
        // created at this point based on the walkdir ordering. If there is a error/panic here it
        // would likely be because there was a file ordering issue.
        //
        // NOTE: We are also using the fact that all images/media will live inside a directory
        // called "{Some Markdown File Name}.assets/{image_asset_path}.{ext}"
        // Therefore we can get to the file's name using only the image's asset path.
        let maybe_md_path_rel = {
            // Example:
            // Hand Tools Woodworking.assets/image.jpeg
            // Hand Tools Woodworking.assets
            let mut asset_dir_path = asset_path.to_path_buf();
            asset_dir_path.pop();
            // Remove the 'assets' extension from the directory name to use as a key
            // Example:
            // Hand Tools Woodworking.assets
            // Hand Tools Woodworking
            asset_dir_path.set_extension("");
            asset_dir_path = strip_input_dir(&asset_dir_path, &self.input_dir)?;
            asset_dir_path
        };

        let mut file_data = self.files.get_mut(&maybe_md_path_rel);
        if let Some(ref mut file) = file_data {
            let file_name_path = {
                let mut asset_dir_path = asset_path.to_path_buf();
                asset_dir_path.pop();
                strip_input_dir(asset_path, &asset_dir_path)?
            };

            // Push the asset_path onto the file_data's assets vec, or create it if one does not
            // exist
            match &mut file.assets {
                Some(a) => a.push(file_name_path),
                None => file.assets = Some(vec![file_name_path]),
            };

            return Ok(());
        }
        println!(
            "Failed to get a matching file in the HashMap for the asset dir name = {} files = {:#?}",
            asset_path.display(),
            self.files
        );
        bail!("Bailed at set_asset_on_file");
    }

    fn set_asset_dir(&mut self, rel_path: PathBuf) -> anyhow::Result<()> {
        // Remove the 'assets' extension from the directory name to use as a key
        let mut maybe_file_path = rel_path.clone();
        maybe_file_path.set_extension("");

        // Find the associated file that matches the name of the asset directory
        let file = self.files.get_mut(&maybe_file_path);
        if let Some(file) = file {
            // If this file has an associated assets directory we will have to co-locate the final
            // markdown file into the same directory.
            //
            // Since the slugified name of the directory and the slugified name of the file should be
            // identical (given the same rules), we can just add "index" to the end of this slugified
            // path and it will become the full path including the directory both will live in.
            //
            // Example:
            //  File: "cryptography/aes.md"
            //  Assets: "cryptography/AES.assets/"
            //  Final File Path: crypography/aes/index.md
            //                               ^ will also be the assets dir
            let file_path_slug = &mut file.path_slug;
            file_path_slug.set_extension("");
            file_path_slug.push("index");
            file_path_slug.set_extension(MD_EXT);

            // Set this directory as the assets_dir on the matching FileData
            // Example: {INPUT_DIR}/Woodworking/DoveTail Joing.assets/
            let abs_asset_dir_path = create_input_path(&self.input_dir, &rel_path);
            file.assets_dir = Some(abs_asset_dir_path);

            return Ok(());
        }
        println!(
            "Tried to get the matching file for this directory but it doesn't exist. File name = {}, files hashmap = {:?}",
            maybe_file_path.display(),
            self.files
        );
        bail!("Failed to get the file for this asset directory")
    }

    // Get all of the files as an interator
    //
    // For each file
    //      Read the contents from the disk into a buffer / string?
    //
    //      Remove the first `#` h1 header. Store it
    //
    //      Format YAML frontmatter string
    //      ---
    //      title: {}
    //      date: {}
    //      updated: {}
    //      weight: {}
    //      ---
    //
    //      Fill yaml frontmatter with values
    //      Write to buffer
    //
    //      Find all [[wiki style links]]
    //      Replace with a markdown style link formatted for Zola
    //      Example: [{original wiki link text}](@/input_dir_last/{file.path_slug})
    //
    //      Find all links with [Mon, Dec 3](day://2023.12.03)
    //      Replace it with a link that goes nowhere
    //      Example: [Mon, Dec 3](.)
    //
    //      Find all image links to media inside '.assets' directories
    //      Replace with the file name only
    //      Example:
    //          ![Image.jpeg](Non%20Qualified%20Stock%20Options(NSO).assets/Image.jpeg)
    //                                              only want this part ----^--------^
    //          ![Image.jpeg](Image.jpeg)
    //
    //      + Renaming/modifying files
    //      If the file has an assets directory
    //          Rename the assets directory (remove '.assets')
    //          Rename the assets directory (slugify the path)
    //          Move the associated markdown file into the assets directory
    //          Rename the markdown file ('index.md')
    //
    //
    pub fn format_markdown(&mut self) -> anyhow::Result<()> {
        let mut files = self.files.clone();
        for (_path_rel, file_data) in files.iter_mut() {
            let mut buffer = read_to_string(&file_data.path_full)?;

            // ERROR - Immediately if we find a buffer which contains a markdown link pointing to a
            // Craft block.
            // From the web, a link pointing to craftdocs://open?blockID={}&spaceID={} will be
            // unusable.
            // Having these links within any document represents an invalid export of the craft
            // workspace.
            if let Some(cap) = RE_CRAFTDOCS_LINK.captures(buffer.as_str()) {
                bail!(
                    "Invalid document: \n
                    File = '{}' \n
                    This document contains a markdown link to an internal or prviate Craft block. \n
                    The link is in the format of ()[craftdocs://open?blockID={{}}&spaceID={{}}] \n
                    Link = '{}'",
                    file_data.path_full.display(),
                    cap.get(0).unwrap().as_str()
                );
            }

            // Replace the first #H1 Header in the file
            // This is because Zola will have the file's `title` in the frontmatter we generate
            // Zola renders the title as an h1 anyway so there is little point in having two titles
            buffer = RE_FIRST_H1.replace(&buffer, "").into();

            // We are going to format the frontmatter for this markdown file and pre-pend it to the
            // existing document in place
            buffer = format!(
                "---\n\
                title: \"{}\"\n\
                date: {}\n\
                updated: {}\n\
                weight: {}\n\
                extra:\n  \
                note_type: {}\n\
                ---\n\
                {}",
                &file_data.name,
                &file_data.created_at,
                file_data.modified_at,
                file_data.note_type.to_weight(),
                file_data.note_type.as_emoji(),
                buffer
            );

            // Find all the [[Wiki Links]] in this buffer and replace them with their
            // Zola internal link equivalent
            buffer = self
                .replace_all(&RE_WIKI_LINK, buffer.as_str(), |caps, m| {
                    self.replace_wiki_link(caps, m)
                })
                .with_context(|| {
                    format!(
                        "Got some invalid [[wiki link]] in file = {}",
                        file_data.path_full.display()
                    )
                })?;

            // Find all the date links and [Tues, Jan 4](day://2023.01.04) and replace link portion
            // with '.'
            buffer = self
                .replace_all(&RE_DAY_LINK, buffer.as_str(), |caps, m| {
                    self.replace_day_link(caps, m)
                })
                .with_context(|| {
                    format!(
                        "Got some invalid [day://yyyy.mm.dd] in file = {}",
                        file_data.path_full.display()
                    )
                })?;

            // Find all image links to media inside '.assets' directories
            // Replace with the file name only
            // Example:
            //  ![Image.jpeg](Non%20Qualified%20Stock%20Options(NSO).assets/Image.jpeg)
            //                                      only want this part ----^--------^
            //  ![Image.jpeg](Image.jpeg)
            buffer = self
                .replace_all(&RE_IMG_ASSET_LINK, buffer.as_str(), |cap, m| {
                    self.replace_img_asset_link(cap, m)
                })
                .with_context(|| {
                    format!(
                        "Tried to parse an image link but it was invalid in file = {}",
                        file_data.path_full.display()
                    )
                })?;

            buffer = self.replace_all(&RE_CODE_BLOCK_OTHER, buffer.as_str(), |cap, m| {
                self.replace_all_code_block_other(cap, m)
            }).with_context(|| {
                    format!("Found a code block with syntax 'other' but could not replace it in file = {}", file_data.path_full.display())
                })?;

            file_data.contents = buffer
        }
        self.files = files;
        Ok(())
    }

    // The reference for this replacement routine comes from the Regex documentation.
    //
    // When writing a replacement routine where any replacement may fail, you will need to write
    // your own routine on top of replace_all to handle each Result.
    //
    // https://docs.rs/regex/latest/regex/struct.Regex.html#method.replace_all
    fn replace_all<E>(
        &self,
        re: &Regex,
        haystack: &str,
        replacement: impl Fn(&Captures, &Match) -> Result<String, E>,
    ) -> Result<String, E> {
        let mut new = String::with_capacity(haystack.len());
        let mut last_match = 0;
        for caps in re.captures_iter(haystack) {
            let m = caps.get(0).unwrap();
            let start_original = m.start();
            let end_original = m.end();
            let before = &haystack[last_match..start_original];

            let rep = &replacement(&caps, &m)?;

            new.push_str(before);
            new.push_str(rep);
            last_match = end_original;
        }
        let after = &haystack[last_match..];
        new.push_str(after);
        Ok(new)
    }

    fn replace_wiki_link(
        &self,
        captures: &Captures,
        origin_match: &Match,
    ) -> anyhow::Result<String> {
        let link_name = captures.name("link_name").context(
            "Matched on a [[wiki link]] but did not get any value inside the brackets [[ ]]",
        )?;

        // Does this [[wiki link]] have a Craft Block-ID? (formatted as UUIDv4)
        // Example: [[Expatriation/Dutch-American Friendship Treaty#^2206D341-3D6E-4F31-B7CF-DD7E3D5D7778]]
        // Remove it (if no match it returns the original str)
        let replaced = RE_UUID_V4.replace(link_name.as_str(), "");
        let mut link_name: &str = replaced.as_ref();

        // Does this [[wiki link]] have a header anchor?
        // Example: [[Weightlifting/Lower Body Exercises/Deadlift Variants#Conventional deadlifts]]
        //
        // Split the header anchor out, slugify it, then add it back into the final zola_link
        // We are left with our file_name which
        // should match into the Files HashMap
        let mut header: Option<String> = None;
        if let Some(h_cap) = RE_HEADER_ANCHOR.captures(link_name) {
            let m = h_cap.name("header").unwrap();
            let header_str = format!("#{}", slugify(m.as_str()));
            header = header_str.into();
            let m = h_cap.name("link_name").unwrap();
            link_name = m.as_str();
        }

        let zola_link = self.make_zola_link(link_name, header).with_context(|| {
                format!(
                    "No such file = {} does not exist in our HashMap.
                    This is probably because this [[wiki link]] is referencing a block inside Craft.
                    Because craft will use `^` as a marker for a block link, we cannot use them in Zola",
                    origin_match.as_str()
                )
            })?;
        Ok(zola_link)
    }

    // Names the capture group "desc" for the date string,
    // "day_url" for everything including day://,
    // and "date" for the actual yyyy.mm.dd
    // static ref RE_DAY_LINK: Regex =
    //     Regex::new(r"\[(?<desc>.*)\]\((?<day_url>day:\/\/(?<date>\d{4}\.\d{2}\.\d{2}))\)").unwrap();
    fn replace_day_link(
        &self,
        captures: &Captures,
        origin_match: &Match,
    ) -> anyhow::Result<String> {
        let date = captures.name("date").context(
            "Matched on a ()[day://yyyy.mm.dd] link but did not get any value for yyyy.mm.dd",
        )?;

        // Parse the yyyy.mm.dd using the time crate into a Date
        // Accepted syntax for this macro can be found in the time.rs book
        // https://time-rs.github.io/book/api/format-description.html
        let origin_format = format_description!("[year].[month].[day]");
        // Then reformat that date object into a string to include the year
        //  "Mon, Jan 3 2023
        let new_format =
            format_description!("[weekday repr:short], [month repr:short] [day padding:none] '[year padding:none repr:last_two]");
        let date_obj = Date::parse(date.as_str(), origin_format)
            .with_context(
                || format!("Unable to parse the day:// URL in our link. match = {} url = {} format = [year].[month].[day]",
                    origin_match.as_str(),
                    date.as_str())
            )?;
        let new_date = date_obj.format(&new_format).with_context(|| {
            format!(
                "Unable to format the original date as the new date for match = {} url = {}",
                origin_match.as_str(),
                date.as_str()
            )
        })?;

        // Since date notes are private and are note exported from Craft, remove the URL from the
        // link
        //  [Monday, Jan 3 2023](.)
        let new_date = format!("[{new_date}](javascript:;)");

        Ok(new_date)
    }

    fn replace_img_asset_link(
        &self,
        captures: &Captures,
        origin_match: &Match,
    ) -> anyhow::Result<String> {
        let name = captures
            .name("name")
            .with_context(|| {
                format!(
                    "Failed to get the image link's name from within the brackets [] on text = {}",
                    origin_match.as_str()
                )
            })?
            .as_str();
        let file_name = captures
            .name("file_name")
            .with_context(|| {
                format!(
                    "Failed to get the image link's name from within the brackets [] on text = {}",
                    origin_match.as_str()
                )
            })?
            .as_str();

        let link = format!("![{name}]({file_name})");
        Ok(link)
    }

    fn replace_all_code_block_other(
        &self,
        _captures: &Captures,
        _origin_match: &Match,
    ) -> anyhow::Result<String> {
        Ok("```".into())
    }

    fn make_zola_link(&self, key: &str, header: Option<String>) -> Option<String> {
        self.files.get::<PathBuf>(&key.into()).map(|file_data| {
            let base_dir_name = slugify(&self.input_dir_name);
            let header = header.unwrap_or_default();
            format!(
                "[{name}](@/{base_dir_name}/{file_path_slug}{header})",
                name = &file_data.name,
                file_path_slug = &file_data.path_slug.display(),
            )
        })
    }
}
