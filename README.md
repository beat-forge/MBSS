# MBSS (Mass BeatSaber Stripper)

MBSS is an application designed to create and maintain your own [beatsaber-stripped](https://github.com/beat-forge/beatsaber-stripped) repository.

This tool serves as a contingency plan for BeatSaber modders who want to ensure they have access to stripped game versions in the event that public repositories become unavailable.

## Setup

1. Clone this repository
2. Ensure you have Rust and Cargo installed
3. Set up the necessary environment variables (see Configuration section)
4. Run `cargo build --release` to compile the application
5. Run the application (see Usage section)

## Usage

When you run MBSS for the first time, it will set up a local Git repository and download essential tools such as `DepotDownloader` and `GenericStripper`.

In the repository, you'll find a `versions.json` file listing the BeatSaber versions to process, formatted as follows:

```json
[
  {
    "version": "1.0.0",
    "manifest": "1234567890123456789012345678901234567890"
  }
]
```

To add a new version, simply append an entry to this array with the appropriate version number and manifest identifier.

> [!IMPORTANT]
> Don't forget to commit the changes to the `versions.json` file to the repository before running the application again.

## Configuration

Before running MBSS, make sure to set up the following environment variables:

- `RUST_LOG`: The logging level for the application (e.g., `info`, `debug`, `trace`) If not set, the application will not log anything.
- `GITHUB_TOKEN`: Your GitHub personal access token (if pushing to a remote repository)
- `STEAM_USERNAME`: Your Steam username (for downloading BeatSaber versions)
- `STEAM_PASSWORD`: Your Steam password (for downloading BeatSaber versions)

You can set these in a `.env` file in the project root or in your system's environment.

## Usage

To run MBSS:

```
cargo run --release
```

The application will:
1. Initialize a local Git repository (if it doesn't exist)
2. Download necessary tools such as [`DepotDownloader`](https://github.com/SteamRE/DepotDownloader) and [`GenericStripper`](https://github.com/beat-forge/GenericStripper)
3. Process each BeatSaber version in the `versions.json` file
4. Create a branch for each version and commit the stripped files
5. Push changes to a remote repository (if configured)

## Contributing

Contributions to MBSS are welcome! Please submit pull requests or open issues on the GitHub repository. Ensure your commits follow conventional commit guidelines.

## Legal Disclaimer

MBSS is designed to be used for personal, non-commercial purposes only. This tool does not distribute any copyrighted content. Users are responsible for ensuring their use of this tool complies with all applicable laws and terms of service.

## License

MBSS is licensed under the GNU Affero General Public License v3.0. See the [LICENSE](LICENSE) file for more information.