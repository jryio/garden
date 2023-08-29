use std::{fs, path::PathBuf};

use anyhow::Context;

use crate::craft_files::CraftDocs;

const DIR_EMOJI: char = '🌳';

pub struct ZolaFiles {
    pub output_dir: PathBuf,
}

impl ZolaFiles {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }
    /// write_files takes CraftDocs and writes the processed files into their intended destination
    /// within the Zola OUTPUT_DIR
    pub fn write_files(&self, craft_docs: CraftDocs) -> anyhow::Result<()> {
        for (_path_rel, file_data) in craft_docs.files.iter() {
            self.create_dir(file_data.path_slug.clone())?;
            let output_path = self.create_output_path(&file_data.path_slug);
            fs::write(&output_path, &file_data.contents)?;

            // If this file has associated images, write them relative to the file (index.md)
            if let Some(assets) = &file_data.assets {
                let mut sibling_file_path_slug = output_path.clone();
                sibling_file_path_slug.pop();
                let abs_asset_dir = file_data.assets_dir.as_ref().expect(
                    "There to be an asset_dir on any file_data which also has Some(Vec<Assets>)",
                );
                for asset in assets {
                    let origin_asset_path = abs_asset_dir.join(asset);
                    let destination_asset_path = sibling_file_path_slug.join(asset);
                    fs::copy(origin_asset_path, destination_asset_path)?;
                }
            }

            // If this file is NOT `index` file_name
            // Then we should create *one* and only *one* "_index.md"
            // for the parent dir to make it a section
            if file_data.path_slug.file_name().unwrap() == "index.md" {
                continue;
            }

            let mut parent_dir_path = file_data.path_slug.clone();
            // Remove the file name to get the slugified path to the directory
            parent_dir_path.pop();
            // Append "_index.md" as a new file path
            parent_dir_path.push("_index.md");
            let section_file_path = self.create_output_path(&parent_dir_path);
            // Does this file already exist (we've done this before?)
            let exists = section_file_path.try_exists().with_context(|| {
                format!(
                    "Attempted to check if '{}' existed on the file system but recieved an error",
                    section_file_path.display()
                )
            })?;
            if exists {
                continue;
            }
            // Create this file with the desired contents
            let mut parent_dir_title = file_data.path_rel.clone();
            // Remove the unslugified file name
            parent_dir_title.pop();
            // Get the name of the parent directory
            let parent_dir_title = parent_dir_title.file_name().unwrap().to_str().unwrap();
            let section_content = format!(
                "+++\n\
            title = \"{DIR_EMOJI} {parent_dir_title}\"\n\
            sort_by = \"weight\"\n\
            insert_anchor_links = \"left\"\n\
            +++"
            );

            // Write the file
            fs::write(&section_file_path, section_content).with_context(|| {
                format!(
                    "Failed to write a section _index.md file at path = {}",
                    section_file_path.display()
                )
            })?;
        }
        // SPECIAL CASE: We assume that there are no markdown files as immediate children of out
        // input_dir. Put another way: all files live inside a folder from the top level directory.
        //
        // Since this is the case we will need to generate a top level `_index.md` to mark the top
        // level directory as a section in Zola.
        //
        // Since I am lazy, I am doing this as a manual special cased step.
        let tld_section_index_md = self.output_dir.join(PathBuf::from("_index.md"));
        let section_content = format!(
            "+++\n\
            title = \"{DIR_EMOJI} Garden\"\n\
            sort_by = \"weight\"\n\
            template = \"garden.html\"\n\
            insert_anchor_links = \"left\"\n\
            +++"
        );

        // Write the file
        fs::write(&tld_section_index_md, section_content).with_context(|| {
            format!(
                "Failed to write a section _index.md file at path = {}",
                tld_section_index_md.display()
            )
        })?;
        Ok(())
    }

    fn create_output_path(&self, file_path: &PathBuf) -> PathBuf {
        self.output_dir.join(file_path)
    }

    /// create_dir will build all necessary directories for
    /// {output_dir}/{path_slug.pop}
    fn create_dir(&self, mut file_path_slug: PathBuf) -> anyhow::Result<()> {
        file_path_slug.pop();
        let output_path = self.create_output_path(&file_path_slug);
        fs::create_dir_all(output_path)?;
        Ok(())
    }
}
