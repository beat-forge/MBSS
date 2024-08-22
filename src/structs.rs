use serde::Deserialize;

#[derive(Deserialize)]
pub struct VersionsFile {
    pub versions: Vec<Version>,
}

#[derive(Deserialize)]
pub struct Version {
    pub version: semver::Version,
    pub manifest: String,
}
