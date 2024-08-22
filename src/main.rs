use anyhow::Result;
use git2::{Branch, BranchType, Commit, ObjectType, Reference, Repository, Signature};
use semver::Version;
use std::fs;
use std::path::Path;
use structs::VersionsFile;
use tracing::{error, info, warn};
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
    info!("Processing version: {}", version.version);

    let download_path = match utils::download_version(version, &tools.depot_downloader).await {
        Ok(path) => {
            info!("Successfully downloaded version to: {:?}", path);
            path
        }
        Err(e) => {
            error!("Failed to download version: {:?}", e);
            return Err(e.into());
        }
    };

    let stripped_path = match utils::strip_version(&download_path, &tools.generic_stripper).await {
        Ok(path) => {
            info!("Successfully stripped version to: {:?}", path);
            path
        }
        Err(e) => {
            error!("Failed to strip version: {:?}", e);
            return Err(e.into());
        }
    };

    let branch = if let Some(prev_version) = previous_version {
        let prev_branch_name = format!("version/{}", prev_version);
        info!("Attempting to find previous branch: {}", prev_branch_name);
        match repo.find_branch(&prev_branch_name, BranchType::Local) {
            Ok(prev_branch) => {
                info!("Found previous branch");
                let prev_commit = prev_branch.get().peel_to_commit()?;
                repo.branch(&branch_name, &prev_commit, true)?
            }
            Err(e) => {
                error!("Failed to find previous branch: {:?}", e);
                return Err(e.into());
            }
        }
    } else {
        info!("No previous version, creating new branch");
        let branch = match repo.find_branch(&branch_name, BranchType::Local) {
            Ok(branch) => {
                info!("Branch already exists: {}", branch_name);
                branch
            }
            Err(_) => {
                info!("Creating new branch: {}", branch_name);
                let head = repo.head()?;
                let commit = head.peel_to_commit()?;
                repo.branch(&branch_name, &commit, false)?
            }
        };
        branch
    };

    info!("Writing version.txt");
    let version_txt_content = format!("{}\n", version.version);
    let version_txt_path = stripped_path.join("version.txt");
    if let Err(e) = fs::write(&version_txt_path, version_txt_content) {
        error!("Failed to write version.txt: {:?}", e);
        return Err(e.into());
    }

    let repo_root = repo.workdir().unwrap();
    info!("Copying files to repository");
    fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
        fs::create_dir_all(&dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.as_ref().join(entry.file_name());
            if ty.is_dir() {
                copy_dir_all(src_path, dst_path)?;
            } else {
                if dst_path.exists() {
                    fs::remove_file(&dst_path)?;
                }
                fs::copy(src_path, dst_path)?;
            }
        }
        Ok(())
    }

    if let Err(e) = copy_dir_all(&stripped_path, repo_root) {
        error!("Failed to copy directory: {:?}", e);
        return Err(e.into());
    }

    info!("Updating git index");
    let mut index = repo.index()?;
    if let Err(e) = index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None) {
        error!("Failed to add files to index: {:?}", e);
        return Err(e.into());
    }
    if let Err(e) = index.write() {
        error!("Failed to write index: {:?}", e);
        return Err(e.into());
    }

    info!("Creating commit");
    let signature = Signature::now("MBSS", "mbss@beatforge.net")?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let parent_commit = branch.get().peel_to_commit()?;

    if let Err(e) = repo.commit(
        Some(&format!("refs/heads/{}", branch_name)),
        &signature,
        &signature,
        &format!("feat: update version to {}", version.version),
        &tree,
        &[&parent_commit],
    ) {
        error!("Failed to create commit: {:?}", e);
        return Err(e.into());
    }

    // Push to remote if it exists
    if let Err(e) = push_to_remote(repo, &branch_name) {
        warn!("Failed to push to remote: {:?}", e);
        // Continue execution even if push fails
    }

    info!(
        "Successfully processed and saved version {}",
        version.version
    );
    Ok(())
}

use std::env;

fn push_to_remote(repo: &Repository, branch_name: &str) -> Result<()> {
    if let Ok(mut remote) = repo.find_remote("origin") {
        info!("Pushing to remote origin");
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            let username = username_from_url.unwrap_or("git");
            let token = env::var("GITHUB_TOKEN").expect("GITHUB_TOKEN not set");
            git2::Cred::userpass_plaintext(username, &token)
        });
        callbacks.push_update_reference(|refname, status| {
            if let Some(msg) = status {
                error!("Failed to push {}: {}", refname, msg);
                Err(git2::Error::from_str(&format!(
                    "Failed to push {}: {}",
                    refname, msg
                )))
            } else {
                info!("Successfully pushed {}", refname);
                Ok(())
            }
        });

        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(callbacks);

        remote.push(
            &[&format!("refs/heads/{}", branch_name)],
            Some(&mut push_options),
        )?;
        Ok(())
    } else {
        info!("No remote origin found, skipping push");
        Ok(())
    }
}

fn initialize_repository(repo_path: &Path) -> Result<Repository> {
    info!("Initializing git repository at {:?}", repo_path);
    let repo = Repository::init(repo_path)?;

    let sig = Signature::now("MBSS", "mbss@beatforge.net")?;

    // Copy assets to the repository root
    copy_assets_to_repo(repo_path)?;

    // Create initial commit
    {
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
        info!("Created initial commit with assets on 'main' branch");
    }

    // Ensure HEAD points to 'main'
    repo.set_head("refs/heads/main")?;

    Ok(repo)
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

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = dotenv::dotenv() {
        warn!("Failed to load .env file: {}", e);
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    info!("Starting MBSS");

    let tools = match utils::download_tools().await {
        Ok(paths) => paths,
        Err(e) => {
            error!("Failed to download tools: {:?}", e);
            return Err(e.into());
        }
    };

    let repo_path = Path::new("versions");
    let repo = match Repository::open(repo_path) {
        Ok(repo) => repo,
        Err(e) => match e.code() {
            git2::ErrorCode::NotFound => initialize_repository(repo_path)?,
            _ => {
                error!("Failed to open git repository: {:?}", e);
                return Err(e.into());
            }
        },
    };

    let versions_path = repo.workdir().unwrap().join("versions.json");
    let versions_file = if versions_path.exists() {
        match serde_json::from_reader::<_, VersionsFile>(std::fs::File::open(&versions_path)?) {
            Ok(versions) => {
                info!("Loaded versions from {:?}", versions_path);
                versions
            }
            Err(e) => {
                error!("Failed to parse versions.json: {:?}", e);
                return Err(e.into());
            }
        }
    } else {
        info!("No versions found, downloading all");
        VersionsFile {
            versions: Vec::new(),
        }
    };

    let mut existing_versions: Vec<Version> = repo
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

            let versions_to_reprocess: Vec<&structs::Version> = versions_file
                .versions
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
            info!(
                "Skipping version {} as it already has a branch",
                version.version
            );
        }
    }

    {
        let head = repo.head()?;
        if let Some(branch_name) = head.shorthand() {
            if branch_name != "main" {
                let obj = head.peel(ObjectType::Commit)?;
                let commit = obj
                    .into_commit()
                    .map_err(|_| git2::Error::from_str("Couldn't find commit"))?;

                repo.branch("main", &commit, true)?;
                repo.set_head("refs/heads/main")?;

                info!("Updated 'main' branch to latest commit");
            }
        }
    }

    info!("MBSS completed successfully");
    Ok(())
}
