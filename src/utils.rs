use anyhow::{Context, Result};
use reqwest::Client;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};
use zip::ZipArchive;

use crate::structs;

pub async fn download_depot_downloader() -> Result<()> {
    let client = Client::new();

    let api_url = "https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest";

    debug!("Fetching latest release info for DepotDownloader");
    let release_info: serde_json::Value = client
        .get(api_url)
        .header("User-Agent", "rust-app")
        .send()
        .await
        .context("Failed to send request for DepotDownloader release info")?
        .json()
        .await
        .context("Failed to parse DepotDownloader release info")?;

    let asset_url = release_info["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"].as_str().unwrap_or("").ends_with(".zip"))
        })
        .and_then(|asset| asset["browser_download_url"].as_str())
        .context("Failed to find zip asset URL for DepotDownloader")?;

    debug!("Downloading DepotDownloader zip file");
    let response = client
        .get(asset_url)
        .send()
        .await
        .context("Failed to download DepotDownloader zip file")?;
    let zip_content = response
        .bytes()
        .await
        .context("Failed to read DepotDownloader zip content")?;

    let bin_dir = Path::new("./bin");
    fs::create_dir_all(bin_dir).context("Failed to create bin directory")?;

    let temp_zip = bin_dir.join("depot_downloader_temp.zip");
    let mut temp_file = File::create(&temp_zip)
        .context("Failed to create temporary zip file for DepotDownloader")?;
    temp_file
        .write_all(&zip_content)
        .context("Failed to write DepotDownloader zip content to temporary file")?;

    debug!("Extracting DepotDownloader zip file");
    let mut archive = ZipArchive::new(
        File::open(&temp_zip).context("Failed to open DepotDownloader zip file for extraction")?,
    )?;
    let target_dir = bin_dir.join("DepotDownloader");
    fs::create_dir_all(&target_dir).context("Failed to create DepotDownloader target directory")?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .context("Failed to access file in DepotDownloader zip archive")?;
        let outpath = target_dir.join(file.mangled_name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .context("Failed to create directory during DepotDownloader extraction")?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).context(
                    "Failed to create parent directory during DepotDownloader extraction",
                )?;
            }
            let mut outfile = File::create(&outpath)
                .context("Failed to create output file during DepotDownloader extraction")?;
            std::io::copy(&mut file, &mut outfile)
                .context("Failed to copy file content during DepotDownloader extraction")?;
        }
    }

    fs::remove_file(temp_zip).context("Failed to remove temporary DepotDownloader zip file")?;

    info!(
        "DepotDownloader has been downloaded and extracted to {:?}",
        target_dir
    );
    Ok(())
}

pub async fn download_generic_stripper() -> Result<()> {
    let client = Client::new();

    let api_url = "https://api.github.com/repos/beat-forge/GenericStripper/releases/latest";

    debug!("Fetching latest release info for GenericStripper");
    let release_info: serde_json::Value = client
        .get(api_url)
        .header("User-Agent", "rust-app")
        .send()
        .await
        .context("Failed to send request for GenericStripper release info")?
        .json()
        .await
        .context("Failed to parse GenericStripper release info")?;

    let asset_url = release_info["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"].as_str().unwrap_or("").ends_with(".zip"))
        })
        .and_then(|asset| asset["browser_download_url"].as_str())
        .context("Failed to find zip asset URL for GenericStripper")?;

    debug!("Downloading GenericStripper zip file");
    let response = client
        .get(asset_url)
        .send()
        .await
        .context("Failed to download GenericStripper zip file")?;
    let zip_content = response
        .bytes()
        .await
        .context("Failed to read GenericStripper zip content")?;

    let bin_dir = Path::new("./bin");
    fs::create_dir_all(bin_dir).context("Failed to create bin directory")?;

    let temp_zip = bin_dir.join("generic_stripper_temp.zip");
    let mut temp_file = File::create(&temp_zip)
        .context("Failed to create temporary zip file for GenericStripper")?;
    temp_file
        .write_all(&zip_content)
        .context("Failed to write GenericStripper zip content to temporary file")?;

    debug!("Extracting GenericStripper zip file");
    let mut archive = ZipArchive::new(
        File::open(&temp_zip).context("Failed to open GenericStripper zip file for extraction")?,
    )?;

    let target_dir = bin_dir.join("GenericStripper");
    fs::create_dir_all(&target_dir).context("Failed to create GenericStripper target directory")?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .context("Failed to access file in GenericStripper zip archive")?;
        let outpath = target_dir.join(file.mangled_name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .context("Failed to create directory during GenericStripper extraction")?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).context(
                    "Failed to create parent directory during GenericStripper extraction",
                )?;
            }
            let mut outfile = File::create(&outpath)
                .context("Failed to create output file during GenericStripper extraction")?;
            std::io::copy(&mut file, &mut outfile)
                .context("Failed to copy file content during GenericStripper extraction")?;
        }
    }

    fs::remove_file(temp_zip).context("Failed to remove temporary GenericStripper zip file")?;

    info!(
        "GenericStripper has been downloaded and extracted to {:?}",
        target_dir
    );
    Ok(())
}

pub struct ToolPaths {
    pub depot_downloader: PathBuf,
    pub generic_stripper: PathBuf,
}

pub async fn download_tools() -> Result<ToolPaths> {
    let bin_dir = Path::new("./bin");
    let depot_downloader_dir = bin_dir.join("DepotDownloader");
    let generic_stripper_dir = bin_dir.join("GenericStripper");

    if !depot_downloader_dir.exists() {
        download_depot_downloader()
            .await
            .context("Failed to download DepotDownloader")?;
    }

    if !generic_stripper_dir.exists() {
        download_generic_stripper()
            .await
            .context("Failed to download GenericStripper")?;
    }

    let depot_downloader_exe = depot_downloader_dir.join("DepotDownloader.exe");
    let generic_stripper_exe = generic_stripper_dir.join("GenericStripper.exe");

    if !depot_downloader_exe.exists() {
        return Err(anyhow::anyhow!(
            "DepotDownloader.exe not found after download"
        ));
    }
    if !generic_stripper_exe.exists() {
        return Err(anyhow::anyhow!(
            "GenericStripper.exe not found after download"
        ));
    }

    Ok(ToolPaths {
        depot_downloader: depot_downloader_exe,
        generic_stripper: generic_stripper_exe,
    })
}

pub async fn download_version(version: &structs::Version, depot_downloader: &Path) -> Result<PathBuf> {
    let download_dir = Path::new("./downloads");
    fs::create_dir_all(download_dir).context("Failed to create downloads directory")?;

    let download_path = download_dir.join(&version.manifest);
    if download_path.exists() {
        warn!("Version {} already downloaded, skipping", version.version);
        return Ok(download_path);
    }

    info!("Downloading version {}", version.version);
    let status = std::process::Command::new(depot_downloader)
        .arg("-app")
        .arg("570")
        .arg("-depot")
        .arg("620980")
        .arg("-manifest")
        .arg(&version.manifest)
        .arg("-dir")
        .arg(&download_dir)
        .status()
        .context("Failed to execute DepotDownloader")?;

    if !status.success() {
        return Err(anyhow::anyhow!("DepotDownloader failed with exit code {:?}", status.code()));
    }
    
    Ok(download_path)
}

pub async fn strip_version(download_path: &Path, generic_stripper: &Path) -> Result<PathBuf> {
    let stripped_dir = Path::new("./stripped");
    fs::create_dir_all(stripped_dir).context("Failed to create stripped directory")?;

    let stripped_path = stripped_dir.join(download_path.file_name().unwrap());
    if stripped_path.exists() {
        warn!("Version already stripped, skipping");
        return Ok(stripped_path);
    }

    info!("Stripping version");
    let status = std::process::Command::new(generic_stripper)
        .arg("strip")
        .arg("-m")
        .arg("beatsaber")
        .arg("-p")
        .arg(download_path)
        .arg("-o")
        .arg(&stripped_dir)
        .status()
        .context("Failed to execute GenericStripper")?;

    if !status.success() {
        return Err(anyhow::anyhow!("GenericStripper failed with exit code {:?}", status.code()));
    }

    Ok(stripped_path)
}