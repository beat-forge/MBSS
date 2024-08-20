using System.Diagnostics;
using System.IO.Compression;
using System.Net;
using System.Text;
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
                AnsiConsole.MarkupLine($"[yellow]Processing version {version.Version}...[/]");

                var branchName = $"version/v{version.Version}";
                if (Repository.IsValid(Directory.GetCurrentDirectory()) && new Repository(Directory.GetCurrentDirectory()).Branches[branchName] != null && !args.Contains("--force"))
                {
                    AnsiConsole.MarkupLine($"[yellow]Branch {branchName} already exists, skipping...[/]");
                    continue;
                }

                var tempDir = Path.Combine(Path.GetTempPath(), Path.GetRandomFileName());
                Directory.CreateDirectory(tempDir);

                try
                {
                    await GetAndStrip(version, tempDir);
                    CommitAndPushVersion(version, branchName, tempDir);
                }
                catch (Exception ex)
                {
                    AnsiConsole.MarkupLine($"[red]Failed to process version {version.Version}: {ex.Message}[/]");
                    continue;
                }
                finally
                {
                    Directory.Delete(tempDir, true);
                }

                AnsiConsole.MarkupLine($"[green]Version {version.Version} processed successfully![/]");
            }

            AnsiConsole.MarkupLine("[green]All versions processed successfully![/]");
        }

        private static void InitConsole()
        {
            AnsiConsole.MarkupLine("[bold yellow]MBSS - Mass Beat Saber Stripper[/]");
            AnsiConsole.MarkupLine("[green]This program will download and strip the Beat Saber versions listed in versions.json.[/]");
            AnsiConsole.MarkupLine("[green]It will then commit and push the stripped versions to the respective branches of the repository.[/]");
            AnsiConsole.MarkupLine("[green]Ensure you are running MBSS inside the root of your desired versions repository![/]");
        }

        private static async Task<bool> LoadAndValidateVersions(string versionsFilePath)
        {
            if (!File.Exists(versionsFilePath))
            {
                AnsiConsole.MarkupLine("[red]versions.json does not exist![/]");
                return false;
            }

            var versions = JsonConvert.DeserializeObject<List<BeatSaberVersion>>(await File.ReadAllTextAsync(versionsFilePath));
            if (versions != null) return true;
            
            AnsiConsole.MarkupLine("[red]Failed to parse versions.json![/]");
            return false;

        }

        private static bool ValidateEnvironmentVariables(string[] envs)
        {
            foreach (var env in envs)
            {
                if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable(env))) continue;
                AnsiConsole.MarkupLine($"[red]Environment variable {env} is not set![/]");
                return false;
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

            if (!File.Exists(DepotDownloaderExe)) await DownloadAndExtract(client, DepotDownloaderUrl, DepotDownloaderExe);
            if (!File.Exists(GenericStripperExe)) await DownloadAndExtract(client, GenericStripperUrl, GenericStripperExe);

            return true;
        }

        private static async Task GetAndStrip(BeatSaberVersion version, string tempDir)
        {
            var depotDownloaderPath = Path.GetFullPath(DepotDownloaderExe);
            var genericStripperPath = Path.GetFullPath(GenericStripperExe);
            var downloadPath = Path.Combine(tempDir, "download");
            var strippedPath = Path.Combine(tempDir, "stripped");

            Directory.CreateDirectory(downloadPath);
            Directory.CreateDirectory(strippedPath);

            await RunProcess(depotDownloaderPath, $"-app 620980 -depot 620981 -manifest \"{version.Manifest}\" -dir \"{downloadPath}\" -remember-password -username \"{Environment.GetEnvironmentVariable("STEAM_USERNAME")}\" -password \"{Environment.GetEnvironmentVariable("STEAM_PASSWORD")}\"");
            await RunProcess(genericStripperPath, $"strip -m beatsaber -p \"{downloadPath}\" -o \"{strippedPath}\"");

            await File.WriteAllTextAsync(Path.Combine(strippedPath, "version.txt"), version.Version);
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

            var outputBuilder = new StringBuilder();
            var errorBuilder = new StringBuilder();

            process.OutputDataReceived += (_, args) =>
            {
                if (args.Data == null) return;
                
                outputBuilder.AppendLine(args.Data);
                var escapedOutput = args.Data.Replace("[", "[[").Replace("]", "]]");
                AnsiConsole.MarkupLine($"[yellow]{escapedOutput}[/]");
            };
            
            process.ErrorDataReceived += (_, args) =>
            {
                if (args.Data == null) return;
                
                errorBuilder.AppendLine(args.Data);
                var escapedError = args.Data.Replace("[", "[[").Replace("]", "]]");
                AnsiConsole.MarkupLine($"[red]{escapedError}[/]");
            };

            process.Start();
            process.BeginOutputReadLine();
            process.BeginErrorReadLine();

            await process.WaitForExitAsync();

            if (process.ExitCode != 0)
            {
                var errorOutput = errorBuilder.ToString();
                AnsiConsole.MarkupLine($"[red]Process {fileName} failed with exit code {process.ExitCode}[/]");
                AnsiConsole.MarkupLine($"[red]Error Output: {errorOutput.Replace("[", "[[").Replace("]", "]]")}[/]");
                throw new Exception($"Process {fileName} failed with exit code {process.ExitCode}. Error Output: {errorOutput}");
            }

            AnsiConsole.MarkupLine($"[green]Process {fileName} completed successfully.[/]");
        }

        private static async Task DownloadAndExtract(HttpClient client, string url, string outputPath)
        {
            AnsiConsole.MarkupLine($"[yellow]{Path.GetFileName(outputPath)} does not exist, downloading...[/]");

            HttpResponseMessage res;
            try
            {
                res = await client.GetAsync(url);
            }
            catch (Exception ex)
            {
                AnsiConsole.MarkupLine($"[red]Error downloading {Path.GetFileName(outputPath)}: {ex.Message}[/]");
                throw;
            }

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

            if (latestRelease["assets"] is not JArray assets || !assets.Any())
            {
                AnsiConsole.MarkupLine("[red]No assets found in the release![/]");
                throw new Exception($"No assets found for {Path.GetFileName(outputPath)}!");
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

            HttpResponseMessage assetRes;
            try
            {
                assetRes = await client.GetAsync(assetUrl);
            }
            catch (Exception ex)
            {
                AnsiConsole.MarkupLine($"[red]Error downloading asset from {assetUrl}: {ex.Message}[/]");
                throw;
            }

            if (assetRes.StatusCode != HttpStatusCode.OK)
            {
                AnsiConsole.MarkupLine("[red]Failed to download the asset![/]");
                throw new Exception($"Failed to download {Path.GetFileName(outputPath)} asset!");
            }

            await using var assetStream = await assetRes.Content.ReadAsStreamAsync();
            using var archive = new ZipArchive(assetStream);

            var extractPath = Path.Combine(Directory.GetCurrentDirectory(), "bin");

            if (!Directory.Exists(extractPath)) Directory.CreateDirectory(extractPath);
            archive.ExtractToDirectory(extractPath, true);

            AnsiConsole.MarkupLine($"[green]{Path.GetFileName(outputPath)} downloaded and extracted successfully![/]");
        }

        private static void CommitAndPushVersion(BeatSaberVersion version, string branchName, string tempDir)
        {
            using var repo = new Repository(Directory.GetCurrentDirectory());

            var signature = new Signature(Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"), Environment.GetEnvironmentVariable("GIT_AUTHOR_EMAIL"), DateTimeOffset.Now);
            var branch = repo.Branches[branchName];

            if (branch == null)
            {
                branch = repo.CreateBranch(branchName);
                repo.Branches.Update(branch, b => b.Remote = "origin", b => b.UpstreamBranch = branch.CanonicalName);
            }

            Commands.Checkout(repo, branch);

            foreach (var file in repo.Index)
            {
                File.Delete(file.Path);
            }

            var strippedPath = Path.Combine(tempDir, "stripped");
            foreach (var file in Directory.GetFiles(strippedPath, "*", SearchOption.AllDirectories))
            {
                var relativePath = Path.GetRelativePath(strippedPath, file);
                var destPath = Path.Combine(repo.Info.WorkingDirectory, relativePath);
                Directory.CreateDirectory(Path.GetDirectoryName(destPath) ?? throw new InvalidOperationException());
                File.Copy(file, destPath, true);
            }

            Commands.Stage(repo, "*");
            repo.Commit($"chore: strip v{version.Version}", signature, signature);

            var pushOptions = new PushOptions
            {
                CredentialsProvider = (_, _, _) => new UsernamePasswordCredentials { Username = Environment.GetEnvironmentVariable("GITHUB_TOKEN"), Password = string.Empty }
            };

            repo.Network.Push(repo.Branches[branchName], pushOptions);
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