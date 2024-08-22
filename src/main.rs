use anyhow::Result;
use git2::{BranchType, Repository, Branch, Commit};
use semver::Version;
use std::fs;
use std::path::Path;
use structs::VersionsFile;
use tracing::{error, info};
use tracing_subscriber::{self, EnvFilter};

pub mod structs;
pub mod utils;

async fn process_version(
    repo: &Repository,
    version: &structs::Version,
    tools: &utils::ToolPaths,
    previous_version: Option<&Version>,
) -> Result<()> {
    let branch_name = format!("version/{}", version.version);
    
    let download_path = utils::download_version(version, &tools.depot_downloader).await?;
    let stripped_path = utils::strip_version(&download_path, &tools.generic_stripper).await?;

    let branch = if let Some(prev_version) = previous_version {
        let prev_branch_name = format!("version/{}", prev_version);
        let prev_branch = repo.find_branch(&prev_branch_name, BranchType::Local)?;
        let prev_commit = prev_branch.get().peel_to_commit()?;
        repo.branch(&branch_name, &prev_commit, true)?
    } else {
        match repo.find_branch(&branch_name, BranchType::Local) {
            Ok(branch) => branch,
            Err(_) => {
                let head = repo.head()?;
                let commit = head.peel_to_commit()?;
                repo.branch(&branch_name, &commit, false)?
            }
        }
    };

    let version_txt_content = format!("{}\n", version.version);
    let version_txt_path = stripped_path.join("version.txt");
    fs::write(&version_txt_path, version_txt_content)?;

    let repo_root = repo.workdir().unwrap();
    for entry in fs::read_dir(&stripped_path)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = repo_root.join(entry.file_name());

        if ty.is_dir() {
            fs::create_dir_all(&dst_path)?;
            for subentry in fs::read_dir(&src_path)? {
                let subentry = subentry?;
                let sub_src_path = subentry.path();
                let sub_dst_path = dst_path.join(subentry.file_name());
                fs::copy(&sub_src_path, &sub_dst_path)?;
            }
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let signature = repo.signature()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent_commit = branch.get().peel_to_commit()?;

    repo.commit(
        Some(&format!("refs/heads/{}", branch_name)),
        &signature,
        &signature,
        &format!("feat: update version to {}", version.version),
        &tree,
        &[&parent_commit],
    )?;

    info!("Processed and saved version {}", version.version);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    info!("Starting MBSS");

    let tools = match utils::download_tools().await {
        Ok(paths) => paths,
        Err(e) => {
            error!("Failed to download tools: {:?}", e);
            return Err(e);
        }
    };

    let repo_path = Path::new("versions");
    let repo = if !repo_path.exists() {
        info!("Initializing git repository at {:?}", repo_path);
        Repository::init(repo_path)?
    } else {
        Repository::open(repo_path)?
    };

    let versions_path = repo.workdir().unwrap().join("versions.json");
    let versions_file = if versions_path.exists() {
        let versions =
            serde_json::from_reader::<_, VersionsFile>(std::fs::File::open(&versions_path)?)?;
        info!("Loaded versions from {:?}", versions_path);
        versions
    } else {
        info!("No versions found, downloading all");
        VersionsFile {
            versions: Vec::new(),
        }
    };

    let mut existing_versions: Vec<Version> = repo.branches(Some(BranchType::Local))?
        .filter_map(|b| {
            b.ok().and_then(|(branch, _)| {
                branch.name().ok().flatten().and_then(|name| {
                    name.strip_prefix("version/")
                        .and_then(|v| Version::parse(v).ok())
                })
            })
        })
        .collect();

    existing_versions.sort();

    for version in &versions_file.versions {
        let previous_version = existing_versions
            .iter()
            .rev()
            .find(|&v| v < &version.version)
            .cloned();

        if !existing_versions.contains(&version.version) {
            process_version(&repo, version, &tools, previous_version.as_ref()).await?;
            existing_versions.push(version.version.clone());
            existing_versions.sort();

            let versions_to_reprocess: Vec<&structs::Version> = versions_file.versions
                .iter()
                .filter(|v| &v.version > &version.version)
                .collect();

            for (i, reprocess_version) in versions_to_reprocess.iter().enumerate() {
                info!("Re-processing version {}", reprocess_version.version);
                let prev_version = if i == 0 { 
                    Some(&version.version) 
                } else { 
                    Some(&versions_to_reprocess[i - 1].version) 
                };
                process_version(&repo, reprocess_version, &tools, prev_version).await?;
            }
        } else {
            info!("Skipping version {} as it already has a branch", version.version);
        }
    }

    info!("MBSS completed successfully");
    Ok(())
}