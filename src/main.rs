mod structs;
mod utils;

use anyhow::{Context, Result};
use git2::{build::CheckoutBuilder, BranchType, IndexAddOption, Repository, Signature};
use include_dir::{include_dir, Dir};
use semver::Version;
use std::path::Path;
use std::{collections::HashSet, path::PathBuf};
use structs::VersionsFile;
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::EnvFilter;
use utils::{download_tools, download_version, strip_version, ToolPaths};

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

    repo.checkout_tree(tree.as_object(), Some(&mut checkout_options))
        .context("Failed to checkout tree")?;

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

    let repo_workdir = repo.workdir().context("Failed to get workdir")?;
    ASSETS.extract(repo_workdir)?;

    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let commit_id = repo.commit(
        Some("refs/heads/main"),
        &signature,
        &signature,
        "feat: initial main branch with configuration",
        &tree,
        &[],
    )?;

    info!("Created main branch with commit: {}", commit_id);
    Ok(())
}

#[instrument(skip(repo))]
fn load_versions_file(repo: &Repository) -> Result<VersionsFile> {
    let main_tree = repo.head()?.peel_to_tree()?;

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
    let mut previous_version: Option<&Version> = None;

    // Fetch all remote branches
    fetch_remote_branches(repo)?;

    for version in versions_file.versions.iter() {
        let branch_name = format!("version/{}", version.version);

        if existing_versions.contains(&version.version) {
            info!("Version {} already exists locally", version.version);
            if let Ok(branch) = repo.find_branch(&branch_name, BranchType::Local) {
                let commit = branch.get().peel_to_commit()?;
                latest_commit_id = Some(commit.id());
                previous_version = Some(&version.version);
                continue;
            }
        }

        // Check if the branch exists on the remote
        if branch_exists_on_remote(repo, &branch_name)? {
            info!(
                "Version {} exists on remote, updating local",
                version.version
            );
            update_local_branch(repo, &branch_name)?;
            if let Ok(branch) = repo.find_branch(&branch_name, BranchType::Local) {
                let commit = branch.get().peel_to_commit()?;
                latest_commit_id = Some(commit.id());
                previous_version = Some(&version.version);
                continue;
            }
        }

        info!("Processing new version: {}", version.version);
        let commit_id = process_version(repo, version, tools, previous_version).await?;
        latest_commit_id = Some(commit_id);
        previous_version = Some(&version.version);
    }

    // Update versions/latest branch
    if let Some(commit_id) = latest_commit_id {
        update_latest_branch(repo, commit_id)?;
    }

    Ok(())
}

#[instrument(skip(repo, version, tools, previous_version))]
async fn process_version(
    repo: &Repository,
    version: &structs::Version,
    tools: &ToolPaths,
    previous_version: Option<&Version>,
) -> Result<git2::Oid> {
    let branch_name = format!("version/{}", version.version);
    info!("Processing version: {}", version.version);

    // Delete the branch if it already exists
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
    let workdir = repo
        .workdir()
        .context("Failed to get workdir")?
        .to_path_buf();
    clear_working_directory(&workdir).await?;

    // Copy files and create version.txt
    copy_files_to_repo(repo, &processed_path).await?;
    write_version_file(&workdir, &version.version.to_string()).await?;

    // Stage all changes
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let signature = Signature::now("MBSS", "mbss@beatforge.net")?;
    let commit_message = format!("feat: create version {}", version.version);

    // Create the commit
    let commit_id = if let Some(prev_version) = previous_version {
        let prev_branch_name = format!("version/{}", prev_version);
        let prev_branch = repo.find_branch(&prev_branch_name, BranchType::Local)?;
        let prev_commit = prev_branch.get().peel_to_commit()?;
        repo.commit(
            None,
            &signature,
            &signature,
            &commit_message,
            &tree,
            &[&prev_commit],
        )?
    } else {
        // For the first version, create a commit without a parent
        repo.commit(None, &signature, &signature, &commit_message, &tree, &[])?
    };

    // Create or update the branch to point to the new commit
    let commit = repo.find_commit(commit_id)?;
    repo.branch(&branch_name, &commit, true)?;

    // Set HEAD to the new branch
    repo.set_head(&format!("refs/heads/{}", branch_name))?;

    push_to_remote(repo, &branch_name)?;

    info!(
        "Successfully processed and saved version {}",
        version.version
    );

    Ok(commit_id)
}

async fn clear_working_directory(workdir: &Path) -> Result<()> {
    let workdir = workdir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        for entry in std::fs::read_dir(&workdir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.file_name().unwrap() != ".git" {
                std::fs::remove_dir_all(path)?;
            } else if path.is_file() {
                std::fs::remove_file(path)?;
            }
        }
        Ok::<(), std::io::Error>(())
    })
    .await??;
    Ok(())
}

#[instrument(skip(path))]
async fn write_version_file(path: &Path, version: &str) -> Result<()> {
    let version_txt_content = format!("{}\n", version);
    let version_txt_path = path.join("version.txt");
    tokio::fs::write(&version_txt_path, version_txt_content)
        .await
        .context("Failed to write version file")?;
    info!("Written version file: {:?}", version_txt_path);
    Ok(())
}

#[instrument(skip(repo, src_path))]
async fn copy_files_to_repo(repo: &Repository, src_path: &Path) -> Result<()> {
    let repo_root = repo
        .workdir()
        .context("Failed to get workdir")?
        .to_path_buf();
    let src_path = src_path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        debug!("Copying files from {:?} to {:?}", src_path, repo_root);
        utils::copy_dir_all(&src_path, &repo_root, &[])?;
        info!("Files copied to repository");
        Ok::<(), anyhow::Error>(())
    })
    .await??;

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

fn fetch_remote_branches(repo: &Repository) -> Result<()> {
    if let Ok(mut remote) = repo.find_remote("origin") {
        info!("Fetching remote branches");
        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.download_tags(git2::AutotagOption::All);
        remote.fetch(&[] as &[&str], Some(&mut fetch_options), None)?;
    }
    Ok(())
}

fn branch_exists_on_remote(repo: &Repository, branch_name: &str) -> Result<bool> {
    if let Ok(remote_branch) =
        repo.find_branch(&format!("origin/{}", branch_name), BranchType::Remote)
    {
        Ok(remote_branch.get().target().is_some())
    } else {
        Ok(false)
    }
}

fn update_local_branch(repo: &Repository, branch_name: &str) -> Result<()> {
    let remote_branch = repo.find_branch(&format!("origin/{}", branch_name), BranchType::Remote)?;
    let remote_commit = remote_branch.get().peel_to_commit()?;

    if let Ok(local_branch) = repo.find_branch(branch_name, BranchType::Local) {
        local_branch
            .into_reference()
            .set_target(remote_commit.id(), "Updating local branch to match remote")?;
    } else {
        repo.branch(branch_name, &remote_commit, false)?;
    }

    Ok(())
}

fn update_latest_branch(repo: &Repository, commit_id: git2::Oid) -> Result<()> {
    let commit = repo.find_commit(commit_id)?;
    if let Ok(branch) = repo.find_branch("versions/latest", BranchType::Local) {
        branch
            .into_reference()
            .set_target(commit_id, "Updating latest version")?;
    } else {
        repo.branch("versions/latest", &commit, true)?;
    }
    push_to_remote(repo, "versions/latest")?;
    Ok(())
}
