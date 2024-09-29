mod structs;
mod utils;

use anyhow::{Context, Result};
use git2::{build::CheckoutBuilder, BranchType, IndexAddOption, Repository, Signature};
use include_dir::{include_dir, Dir};
use semver::Version;
use std::fs;
use std::path::Path;
use std::{collections::HashSet, path::PathBuf};
use tempfile::TempDir;
use structs::VersionsFile;
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::EnvFilter;
use utils::{copy_dir_all, download_tools, download_version, strip_version, ToolPaths};

static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets");

#[tokio::main]
#[instrument]
async fn main() -> Result<()> {
    initialize_environment()?;
    info!("Starting MBSS");

    let tools = download_tools().await.context("Failed to download tools")?;

    let repo_path = std::env::var("REPO_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./versions"));

    let repo = initialize_repository(&repo_path)?;
    info!("Repository initialized at {:?}", repo.path());

    if let Ok(_) = repo.find_branch("main", BranchType::Local) {
        info!("Main branch already exists, skipping creation");
    } else {
        create_main_branch(&repo)?;
        info!("Main branch created with assets");
    }

    checkout_main_branch(&repo)?;

    let versions_file = load_versions_file(&repo)?;
    info!(
        "Versions file loaded with {} versions",
        versions_file.versions.len()
    );

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    process_versions(&repo, &versions_file, &tools).await?;

    info!("MBSS completed successfully");

    checkout_main_branch(&repo)?;

    Ok(())
}

#[instrument(skip(repo))]
fn checkout_main_branch(repo: &Repository) -> Result<()> {
    let main_branch = repo
        .find_branch("main", git2::BranchType::Local)
        .context("Failed to find main branch")?;

    let main_commit = main_branch
        .get()
        .peel_to_commit()
        .context("Failed to peel to commit")?;

    let tree = main_commit
        .tree()
        .context("Failed to get tree from commit")?;

    let mut checkout_options = CheckoutBuilder::new();
    checkout_options
        .force()
        .remove_untracked(true)
        .remove_ignored(true)
        .conflict_style_merge(true)
        .use_ours(true);

    match repo.checkout_tree(tree.as_object(), Some(&mut checkout_options)) {
        Ok(_) => {}
        Err(e) => {
            warn!("Checkout encountered issues: {}", e);
            checkout_options.force();
            repo.checkout_tree(tree.as_object(), Some(&mut checkout_options))
                .context("Failed to force checkout tree")?;
            warn!("Forced checkout completed. Some local changes may have been overwritten.");
        }
    }

    repo.set_head("refs/heads/main")
        .context("Failed to set HEAD to main branch")?;

    info!("Successfully checked out main branch");
    Ok(())
}

#[instrument]
fn initialize_environment() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    info!("Environment initialized");
    Ok(())
}

#[instrument(skip(repo_path))]
fn initialize_repository(repo_path: &Path) -> Result<Repository> {
    let repo = if repo_path.exists() && repo_path.join(".git").exists() {
        info!("Opened existing repository at {:?}", repo_path);
        Repository::open(repo_path)?
    } else {
        info!("Initializing new git repository at {:?}", repo_path);
        std::fs::create_dir_all(repo_path)?;
        Repository::init(repo_path)?
    };
    Ok(repo)
}

#[instrument(skip(repo))]
fn create_main_branch(repo: &Repository) -> Result<()> {
    let signature = Signature::now("MBSS", "mbss@beatforge.net")?;
    let tree_id = {
        let mut index = repo.index()?;
        let temp_dir = TempDir::new()?;
        let dest_path = temp_dir.path();

        // Extract bundled assets to the temporary directory
        ASSETS.extract(dest_path)?;

        let repo_workdir = repo.workdir().context("Failed to get workdir")?;
        copy_dir_all(dest_path, repo_workdir, &[])?;

        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
        index.write_tree()?
    };

    let tree = repo.find_tree(tree_id)?;
    let commit_id = repo.commit(
        Some("refs/heads/main"),
        &signature,
        &signature,
        "feat: initial main branch",
        &tree,
        &[],
    )?;

    info!("Created main branch with commit: {}", commit_id);
    Ok(())
}

#[instrument(skip(repo))]
fn load_versions_file(repo: &Repository) -> Result<VersionsFile> {
    let main_branch = repo.find_branch("main", git2::BranchType::Local)?;
    let main_commit = main_branch.get().peel_to_commit()?;
    let main_tree = main_commit.tree()?;

    let versions_json_entry = main_tree
        .get_name("versions.json")
        .context("versions.json not found in main branch")?;
    let versions_json_blob = repo.find_blob(versions_json_entry.id())?;
    let content = versions_json_blob.content();

    let versions_file: VersionsFile =
        serde_json::from_slice(content).context("Failed to parse versions.json")?;
    info!(
        "Loaded versions file from main branch with {} versions",
        versions_file.versions.len()
    );
    Ok(versions_file)
}

#[instrument(skip(repo, versions_file, tools))]
async fn process_versions(
    repo: &Repository,
    versions_file: &VersionsFile,
    tools: &ToolPaths,
) -> Result<()> {
    let existing_versions: HashSet<Version> = get_existing_versions(repo)?.into_iter().collect();
    info!("Found {} existing versions", existing_versions.len());

    let mut latest_commit_id = None;

    for (index, version) in versions_file.versions.iter().enumerate() {
        let parent_branch_name = if index == 0 {
            None
        } else {
            Some(format!(
                "version/{}",
                versions_file.versions[index - 1].version
            ))
        };

        if !existing_versions.contains(&version.version) {
            info!("Processing new version: {}", version.version);
            let commit_id =
                process_version(repo, version, tools, parent_branch_name.as_deref()).await?;
            latest_commit_id = Some(commit_id);
        } else {
            info!("Skipping version {} as it already exists", version.version);

            // Get the latest commit id
            let branch_name = format!("version/{}", version.version);
            let branch = repo.find_branch(&branch_name, BranchType::Local)?;
            let commit = branch.get().peel_to_commit()?;
            latest_commit_id = Some(commit.id());
        }
    }

    // Update versions/latest branch
    if let Some(commit_id) = latest_commit_id {
        let commit = repo.find_commit(commit_id)?;
        if let Ok(mut branch) = repo.find_branch("versions/latest", BranchType::Local) {
            branch.delete()?;
        }
        repo.branch("versions/latest", &commit, true)?;
        push_to_remote(repo, "versions/latest")?;
    }

    Ok(())
}

#[instrument(skip(repo, version, tools))]
async fn process_version(
    repo: &Repository,
    version: &structs::Version,
    tools: &ToolPaths,
    parent_branch_name: Option<&str>,
) -> Result<git2::Oid> {
    let branch_name = format!("version/{}", version.version);
    info!("Processing version: {}", version.version);

    // Before deleting the branch, ensure it's not the current HEAD
    let head = repo.head()?;
    let current_branch = head.shorthand().unwrap_or("");
    if current_branch == branch_name {
        // Checkout main branch before deleting
        checkout_main_branch(repo)?;
    }

    if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
        info!("Deleting existing branch {}", branch_name);
        branch.delete()?;
    }

    let download_path = download_version(version, &tools.depot_downloader).await?;
    info!(
        "Version {} downloaded to {:?}",
        version.version, download_path
    );

    #[cfg(feature = "stripping")]
    let processed_path = {
        let stripped_path = strip_version(&download_path, &tools.generic_stripper).await?;
        info!(
            "Version {} stripped to {:?}",
            version.version, stripped_path
        );
        stripped_path
    };

    #[cfg(not(feature = "stripping"))]
    let processed_path = {
        info!(
            "Stripping is disabled. Using downloaded path {:?} for version {}",
            download_path, version.version
        );
        download_path
    };

    // Clear the working directory
    let workdir = repo.workdir().context("Failed to get workdir")?;
    for entry in fs::read_dir(workdir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.file_name().unwrap() != ".git" {
            fs::remove_dir_all(path)?;
        } else if path.is_file() {
            fs::remove_file(path)?;
        }
    }

    copy_files_to_repo(repo, &processed_path)?;
    write_version_file(workdir, &version.version.to_string())?;

    // Stage all changes
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let signature = Signature::now("MBSS", "mbss@beatforge.net")?;

    let commit_message = format!("feat: update version to {}", version.version);

    // Determine parent commits
    let parents = if let Some(parent_branch_name) = parent_branch_name {
        let parent_branch = repo.find_branch(parent_branch_name, BranchType::Local)?;
        let parent_commit = parent_branch.get().peel_to_commit()?;
        vec![parent_commit]
    } else {
        vec![]
    };

    // Create the commit
    let commit_id = repo.commit(
        None,
        &signature,
        &signature,
        &commit_message,
        &tree,
        &parents.iter().collect::<Vec<_>>(),
    )?;

    // Create the branch pointing to the new commit
    let commit = repo.find_commit(commit_id)?;
    repo.branch(&branch_name, &commit, true)?;

    // Checkout the new branch
    let mut checkout_options = CheckoutBuilder::new();
    checkout_options.force();
    repo.checkout_tree(commit.as_object(), Some(&mut checkout_options))?;
    repo.set_head(&format!("refs/heads/{}", branch_name))?;

    push_to_remote(repo, &branch_name)?;

    info!(
        "Successfully processed and saved version {}",
        version.version
    );

    Ok(commit_id)
}

#[instrument(skip(path))]
fn write_version_file(path: &Path, version: &str) -> Result<()> {
    let version_txt_content = format!("{}\n", version);
    let version_txt_path = path.join("version.txt");
    fs::write(&version_txt_path, version_txt_content)?;
    info!("Written version file: {:?}", version_txt_path);
    Ok(())
}

#[instrument(skip(repo, src_path))]
fn copy_files_to_repo(repo: &Repository, src_path: &Path) -> Result<()> {
    let repo_root = repo.workdir().context("Failed to get workdir")?;
    debug!("Copying files from {:?} to {:?}", src_path, repo_root);
    utils::copy_dir_all(src_path, repo_root, &[])?;
    info!("Files copied to repository");
    Ok(())
}

#[instrument(skip(repo))]
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
    info!("Retrieved {} existing versions", versions.len());
    Ok(versions)
}

#[instrument(skip(repo))]
fn push_to_remote(repo: &Repository, branch_name: &str) -> Result<()> {
    if let Ok(mut remote) = repo.find_remote("origin") {
        info!("Pushing {} to remote origin", branch_name);
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            let username = username_from_url.unwrap_or("git");
            let token = std::env::var("GITHUB_TOKEN")
                .context("GITHUB_TOKEN not set")
                .unwrap();
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

        let refspec = format!("+refs/heads/{}", branch_name);
        remote.push(&[&refspec], Some(&mut push_options))?;
    } else {
        info!("No remote origin found, skipping push");
    }
    Ok(())
}
