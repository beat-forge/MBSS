use anyhow::{Context, Result};
use git2::{BranchType, ObjectType, Repository, Signature};
use semver::Version;
use std::fs;
use std::path::Path;
use tracing::{error, info, warn};
use tracing_subscriber::{self, EnvFilter};

mod structs;
mod utils;

use structs::VersionsFile;
use utils::{ToolPaths, download_tools, download_version, strip_version};

#[tokio::main]
async fn main() -> Result<()> {
    initialize_environment()?;
    info!("Starting MBSS");

    let tools = download_tools().await.context("Failed to download tools")?;
    let repo = initialize_repository(Path::new("versions"))?;
    let versions_file = load_versions_file(&repo)?;

    process_versions(&repo, &versions_file, &tools).await?;
    update_main_branch(&repo)?;

    info!("MBSS completed successfully");
    Ok(())
}

fn initialize_environment() -> Result<()> {
    if let Err(e) = dotenv::dotenv() {
        warn!("Failed to load .env file: {}", e);
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    Ok(())
}

fn initialize_repository(repo_path: &Path) -> Result<Repository> {
    match Repository::open(repo_path) {
        Ok(repo) => Ok(repo),
        Err(e) if e.code() == git2::ErrorCode::NotFound => {
            info!("Initializing new git repository at {:?}", repo_path);
            let repo = Repository::init(repo_path)?;
            create_initial_commit(&repo)?;
            Ok(repo)
        }
        Err(e) => Err(e.into()),
    }
}

fn create_initial_commit(repo: &Repository) -> Result<()> {
    let sig = Signature::now("MBSS", "mbss@beatforge.net")?;
    copy_assets_to_repo(repo.workdir().unwrap())?;

    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    repo.commit(
        Some("refs/heads/main"),
        &sig,
        &sig,
        "chore: initial commit with assets",
        &tree,
        &[],
    )?;
    repo.set_head("refs/heads/main")?;
    info!("Created initial commit with assets on 'main' branch");
    Ok(())
}

fn copy_assets_to_repo(repo_path: &Path) -> Result<()> {
    let assets_path = Path::new("assets");
    if assets_path.exists() && assets_path.is_dir() {
        for entry in fs::read_dir(assets_path)? {
            let entry = entry?;
            let source = entry.path();
            let destination = repo_path.join(entry.file_name());
            if source.is_file() {
                fs::copy(&source, &destination)?;
            }
        }
        info!("Copied assets to repository root");
    } else {
        warn!("Assets folder not found or is not a directory");
    }
    Ok(())
}

fn load_versions_file(repo: &Repository) -> Result<VersionsFile> {
    let versions_path = repo.workdir().unwrap().join("versions.json");
    if versions_path.exists() {
        let file = std::fs::File::open(&versions_path)?;
        serde_json::from_reader(file).context("Failed to parse versions.json")
    } else {
        info!("No versions found, starting with empty list");
        Ok(VersionsFile { versions: Vec::new() })
    }
}

async fn process_versions(repo: &Repository, versions_file: &VersionsFile, tools: &ToolPaths) -> Result<()> {
    let mut existing_versions = get_existing_versions(repo)?;

    for version in &versions_file.versions {
        if !existing_versions.contains(&version.version) {
            let previous_version = existing_versions.iter().rev().find(|&v| v < &version.version);
            process_version(repo, version, tools, previous_version).await?;
            existing_versions.push(version.version.clone());
            existing_versions.sort();

            reprocess_later_versions(repo, versions_file, tools, &version.version).await?;
        } else {
            info!("Skipping version {} as it already has a branch", version.version);
        }
    }
    Ok(())
}

fn get_existing_versions(repo: &Repository) -> Result<Vec<Version>> {
    let mut versions: Vec<Version> = repo
        .branches(Some(BranchType::Local))?
        .filter_map(|b| {
            b.ok().and_then(|(branch, _)| {
                branch.name().ok().flatten().and_then(|name| {
                    name.strip_prefix("version/")
                        .and_then(|v| Version::parse(v).ok())
                })
            })
        })
        .collect();
    versions.sort();
    Ok(versions)
}

async fn process_version(
    repo: &Repository,
    version: &structs::Version,
    tools: &ToolPaths,
    previous_version: Option<&Version>,
) -> Result<()> {
    let branch_name = format!("version/{}", version.version);
    info!("Processing version: {}", version.version);

    let download_path = download_version(version, &tools.depot_downloader).await?;
    let stripped_path = strip_version(&download_path, &tools.generic_stripper).await?;

    create_or_update_branch(repo, &branch_name, previous_version)?;
    write_version_file(&stripped_path, &version.version.to_string())?;
    copy_files_to_repo(repo, &stripped_path)?;
    create_commit(repo, &branch_name, &version.version.to_string())?;
    push_to_remote(repo, &branch_name)?;

    info!("Successfully processed and saved version {}", version.version);
    Ok(())
}

fn create_or_update_branch(repo: &Repository, branch_name: &str, previous_version: Option<&Version>) -> Result<()> {
    if let Some(prev_version) = previous_version {
        let prev_branch_name = format!("version/{}", prev_version);
        let prev_branch = repo.find_branch(&prev_branch_name, BranchType::Local)?;
        let prev_commit = prev_branch.get().peel_to_commit()?;
        repo.branch(branch_name, &prev_commit, true)?;
    } else {
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch(branch_name, &commit, false)?;
    }
    Ok(())
}

fn write_version_file(path: &Path, version: &str) -> Result<()> {
    let version_txt_content = format!("{}\n", version);
    let version_txt_path = path.join("version.txt");
    fs::write(&version_txt_path, version_txt_content)?;
    Ok(())
}

fn copy_files_to_repo(repo: &Repository, src_path: &Path) -> Result<()> {
    let repo_root = repo.workdir().unwrap();
    utils::copy_dir_all(src_path, repo_root)?;
    Ok(())
}

fn create_commit(repo: &Repository, branch_name: &str, version: &str) -> Result<()> {
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = Signature::now("MBSS", "mbss@beatforge.net")?;
    let parent_commit = repo.find_branch(branch_name, BranchType::Local)?.get().peel_to_commit()?;

    repo.commit(
        Some(&format!("refs/heads/{}", branch_name)),
        &signature,
        &signature,
        &format!("feat: update version to {}", version),
        &tree,
        &[&parent_commit],
    )?;
    Ok(())
}

fn push_to_remote(repo: &Repository, branch_name: &str) -> Result<()> {
    if let Ok(mut remote) = repo.find_remote("origin") {
        info!("Pushing to remote origin");
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            let username = username_from_url.unwrap_or("git");
            let token = std::env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN not set");
            git2::Cred::userpass_plaintext(username, &token)
        });
        callbacks.push_update_reference(|refname, status| {
            if let Some(msg) = status {
                error!("Failed to push {}: {}", refname, msg);
                Err(git2::Error::from_str(msg))
            } else {
                info!("Successfully pushed {}", refname);
                Ok(())
            }
        });

        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(callbacks);

        remote.push(&[&format!("refs/heads/{}", branch_name)], Some(&mut push_options))?;
    } else {
        info!("No remote origin found, skipping push");
    }
    Ok(())
}

async fn reprocess_later_versions(repo: &Repository, versions_file: &VersionsFile, tools: &ToolPaths, current_version: &Version) -> Result<()> {
    let versions_to_reprocess: Vec<&structs::Version> = versions_file
        .versions
        .iter()
        .filter(|v| &v.version > current_version)
        .collect();

    for (i, reprocess_version) in versions_to_reprocess.iter().enumerate() {
        info!("Re-processing version {}", reprocess_version.version);
        let prev_version = if i == 0 {
            Some(current_version)
        } else {
            Some(&versions_to_reprocess[i - 1].version)
        };
        process_version(repo, reprocess_version, tools, prev_version).await?;
    }
    Ok(())
}

fn update_main_branch(repo: &Repository) -> Result<()> {
    let head = repo.head()?;
    if let Some(branch_name) = head.shorthand() {
        if branch_name != "main" {
            let commit = head.peel_to_commit()?;
            repo.branch("main", &commit, true)?;
            repo.set_head("refs/heads/main")?;
            info!("Updated 'main' branch to latest commit");
        }
    }
    Ok(())
}