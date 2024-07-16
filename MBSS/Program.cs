using System.Diagnostics;
using System.IO.Compression;
using System.Net;
using LibGit2Sharp;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Spectre.Console;

namespace MBSS
{
    internal class BeatSaberVersion
    {
        [JsonProperty("version")]
        public string Version { get; set; } = string.Empty;

        [JsonProperty("manifest")]
        public string Manifest { get; set; } = string.Empty;
    }

    internal abstract class Program
    {
        private const string UserAgent = "MBSS";
        private const string DepotDownloaderUrl = "https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest";
        private const string GenericStripperUrl = "https://api.github.com/repos/beat-forge/GenericStripper/releases/latest";
        private const string EnvFile = ".env";
        private const string VersionsFile = "versions.json";
        private const string DepotDownloaderExe = "bin/DepotDownloader.exe";
        private const string GenericStripperExe = "bin/GenericStripper.exe";
        private static readonly string[] RequiredEnvs = { "STEAM_USERNAME", "STEAM_PASSWORD", "GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GITHUB_TOKEN" };

        public static async Task Main(string[] args)
        {
            InitConsole();

            using var client = new HttpClient();
            client.DefaultRequestHeaders.Add("User-Agent", UserAgent);

            HandleResetArgument(args);

            if (!await LoadAndValidateVersions(VersionsFile)) return;

            if (File.Exists(EnvFile)) await SetupDotEnv();

            if (!ValidateEnvironmentVariables(RequiredEnvs)) return;

            if (!await PerformPreflightChecks(client)) return;

            var versionsJson = await File.ReadAllTextAsync(VersionsFile);
            var versions = JsonConvert.DeserializeObject<List<BeatSaberVersion>>(versionsJson);
            if (versions == null)
            {
                AnsiConsole.MarkupLine("[red]Failed to parse versions.json![/]");
                return;
            }

            foreach (var version in versions)
            {
                var downloadPath = Path.Combine("downloads", version.Version);
                var versionPath = Path.Combine(Directory.GetCurrentDirectory(), "data");

                var branchName = $"versions/{version.Version}";
                if (Directory.Exists(versionPath) &&
                    File.ReadAllText(Path.Combine(versionPath, "version.txt")).Trim() == version.Version)
                {
                    AnsiConsole.MarkupLine($"[yellow]Version {version.Version} already exists in branch {branchName}, skipping...[/]");
                    continue;
                }

                await GetAndStrip(version, downloadPath, versionPath);
                AnsiConsole.MarkupLine($"[green]Version {version.Version} stripped![/]");

                await CommitAndPushVersion(version, branchName, versionPath);
            }
        }

        private static void InitConsole()
        {
            AnsiConsole.MarkupLine("[bold yellow]MBSS - Mass Beat Saber Stripper[/]");
            AnsiConsole.MarkupLine("[green]This program will download and strip the Beat Saber versions listed in versions.json.[/]");
            AnsiConsole.MarkupLine("[green]It will then commit and push the stripped versions to the respective branches of the repository.[/]");
            AnsiConsole.MarkupLine("[green]Ensure you are running MBSS inside the root of your desired versions repository![/]");
        }

        private static void HandleResetArgument(string[] args)
        {
            if (args.Length > 0 && args[0] == "--reset")
            {
                AnsiConsole.MarkupLine("[red]Resetting MBSS and deleting all files...[/]");
                DeleteDirectoryIfExists("downloads");
                DeleteDirectoryIfExists("bin");
            }
        }

        private static void DeleteDirectoryIfExists(string path)
        {
            if (Directory.Exists(path)) Directory.Delete(path, true);
        }

        private static async Task<bool> LoadAndValidateVersions(string versionsFilePath)
        {
            if (!File.Exists(versionsFilePath))
            {
                AnsiConsole.MarkupLine("[red]versions.json does not exist![/]");
                return false;
            }

            var versions = JsonConvert.DeserializeObject<List<BeatSaberVersion>>(await File.ReadAllTextAsync(versionsFilePath));
            if (versions == null)
            {
                AnsiConsole.MarkupLine("[red]Failed to parse versions.json![/]");
                return false;
            }

            return true;
        }

        private static bool ValidateEnvironmentVariables(string[] envs)
        {
            foreach (var env in envs)
            {
                if (string.IsNullOrEmpty(Environment.GetEnvironmentVariable(env)))
                {
                    AnsiConsole.MarkupLine($"[red]Environment variable {env} is not set![/]");
                    return false;
                }
            }
            return true;
        }

        private static async Task<bool> PerformPreflightChecks(HttpClient client)
        {
            if (!Repository.IsValid(Directory.GetCurrentDirectory()))
            {
                AnsiConsole.MarkupLine("[red]MBSS is not running inside a Git repository, aborting.[/]");
                return false;
            }

            if (!File.Exists(".gitignore"))
            {
                AnsiConsole.MarkupLine("[red]Git repository does not have a .gitignore, aborting.[/]");
                AnsiConsole.MarkupLine("[red]It is absolutely necessary to ignore the bin/ and downloads/ directories![/]");
                return false;
            }

            if (!File.Exists(DepotDownloaderExe)) await DownloadAndExtract(client, DepotDownloaderUrl, DepotDownloaderExe);
            if (!File.Exists(GenericStripperExe)) await DownloadAndExtract(client, GenericStripperUrl, GenericStripperExe);

            EnsureDirectoryExists("downloads");

            return true;
        }

        private static void EnsureDirectoryExists(string path)
        {
            var dir = new DirectoryInfo(path);
            if (!dir.Exists) dir.Create();
        }

        private static async Task DownloadAndExtract(HttpClient client, string url, string outputPath)
        {
            AnsiConsole.MarkupLine($"[yellow]{Path.GetFileName(outputPath)} does not exist, downloading...[/]");

            var res = await client.GetAsync(url);
            if (res.StatusCode != HttpStatusCode.OK)
            {
                AnsiConsole.MarkupLine("[red]Failed to get the latest release![/]");
                throw new Exception($"Failed to get {Path.GetFileName(outputPath)} release!");
            }

            var latestRelease = JsonConvert.DeserializeObject<Dictionary<string, dynamic>>(await res.Content.ReadAsStringAsync());
            if (latestRelease == null)
            {
                AnsiConsole.MarkupLine("[red]Failed to parse the latest release![/]");
                throw new Exception($"Failed to parse {Path.GetFileName(outputPath)} release!");
            }

            var assets = latestRelease["assets"] as JArray;
            if (assets == null || !assets.Any())
            {
                AnsiConsole.MarkupLine("[red]No assets found in the release![/]");
                throw new Exception($"No assets found for {Path.GetFileName(outputPath)}!");
            }

            AnsiConsole.MarkupLine("[yellow]Available assets:[/]");
            foreach (var asset in assets)
            {
                AnsiConsole.MarkupLine($"- {asset["name"]?.ToString()}");
            }

            var assetItem = assets.FirstOrDefault(x => x["name"]?.ToString().Contains("windows-x64") ?? false);
            if (assetItem == null)
            {
                AnsiConsole.MarkupLine("[red]Failed to find a suitable asset for the current system![/]");
                throw new Exception($"Failed to find a suitable asset for {Path.GetFileName(outputPath)}!");
            }

            var assetUrl = assetItem["browser_download_url"]?.ToString();
            if (string.IsNullOrEmpty(assetUrl))
            {
                AnsiConsole.MarkupLine("[red]Asset URL is empty![/]");
                throw new Exception($"Asset URL is empty for {Path.GetFileName(outputPath)}!");
            }

            var assetRes = await client.GetAsync(assetUrl);
            if (assetRes.StatusCode != HttpStatusCode.OK)
            {
                AnsiConsole.MarkupLine("[red]Failed to download the asset![/]");
                throw new Exception($"Failed to download {Path.GetFileName(outputPath)} asset!");
            }

            await using var assetStream = await assetRes.Content.ReadAsStreamAsync();
            using var archive = new ZipArchive(assetStream);
            archive.ExtractToDirectory(Path.Combine(Directory.GetCurrentDirectory(), "bin"));

            AnsiConsole.MarkupLine($"[green]{Path.GetFileName(outputPath)} downloaded and extracted successfully![/]");
        }

        private static async Task GetAndStrip(BeatSaberVersion version, string downloadPath, string versionPath)
        {
            await RunProcess(DepotDownloaderExe, $"-app 620980 -depot 620981 -manifest \"{version.Manifest}\" -dir {downloadPath} -remember-password -username \"{Environment.GetEnvironmentVariable("STEAM_USERNAME")}\" -password \"{Environment.GetEnvironmentVariable("STEAM_PASSWORD")}\"");
            await RunProcess(GenericStripperExe, $"strip -m beatsaber -p \"{downloadPath}\" -o \"{versionPath}\"");

            if (Directory.Exists(downloadPath)) Directory.Delete(downloadPath, true);
            
            await File.WriteAllTextAsync(Path.Combine(versionPath, "version.txt"), version.Version);
        }

        private static async Task RunProcess(string fileName, string arguments)
        {
            var process = new Process
            {
                StartInfo = new ProcessStartInfo
                {
                    FileName = fileName,
                    Arguments = arguments,
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    UseShellExecute = false,
                    CreateNoWindow = true
                }
            };

            process.Start();
            await process.WaitForExitAsync();
        }

        private static async Task CommitAndPushVersion(BeatSaberVersion version, string branchName, string versionPath)
        {
            using var repo = new Repository(Directory.GetCurrentDirectory());
            var author = new Signature(Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"), Environment.GetEnvironmentVariable("GIT_AUTHOR_EMAIL"), DateTimeOffset.Now);

            var branch = repo.Branches[branchName] ?? repo.CreateBranch(branchName);
            Commands.Checkout(repo, branch);

            var status = repo.RetrieveStatus();
            if (!status.IsDirty) return;

            Commands.Stage(repo, "*");
            repo.Commit($"chore: v{version.Version}", author, author);

            var remote = repo.Network.Remotes["origin"];
            var options = new PushOptions
            {
                CredentialsProvider = (_, _, _) => new UsernamePasswordCredentials
                {
                    Username = Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"),
                    Password = Environment.GetEnvironmentVariable("GITHUB_TOKEN")
                }
            };

            if (remote != null) repo.Network.Push(remote, $"refs/heads/{branchName}", options);
        }

        private static async Task SetupDotEnv()
        {
            var dotenv = await File.ReadAllLinesAsync(EnvFile);
            foreach (var env in dotenv)
            {
                var split = env.Split('=', StringSplitOptions.RemoveEmptyEntries);
                if (split.Length != 2) continue;
                Environment.SetEnvironmentVariable(split[0], split[1]);
            }
        }
    }
}
