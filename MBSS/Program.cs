using System.Diagnostics;
using System.IO.Compression;
using System.Net;
using LibGit2Sharp;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Spectre.Console;

namespace MBSS;

internal class BeatSaberVersion
{
    [JsonProperty("version")] public string Version { get; set; } = string.Empty;
    [JsonProperty("manifest")] public string Manifest { get; set; } = string.Empty;
}

internal abstract class Program
{
    public static async Task Main()
    {
        InitConsole();

        var client = new HttpClient();
        client.DefaultRequestHeaders.Add("User-Agent", "MBSS");

        #region Versions

        if (!File.Exists("versions.json"))
        {
            AnsiConsole.MarkupLine("[red]versions.json does not exist![/]");
            return;
        }

        var versions =
            JsonConvert.DeserializeObject<List<BeatSaberVersion>>(await File.ReadAllTextAsync("versions.json"));
        if (versions == null)
        {
            AnsiConsole.MarkupLine("[red]Failed to parse versions.json![/]");
            return;
        }

        #endregion

        #region Environment Variables

        if (File.Exists(".env")) await SetupDotEnv();

        var envs = new[] { "STEAM_USERNAME", "STEAM_PASSWORD", "GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GITHUB_TOKEN" };
        foreach (var env in envs.Where(env => string.IsNullOrEmpty(Environment.GetEnvironmentVariable(env))))
        {
            AnsiConsole.MarkupLine($"[red]Environment variable {env} is not set![/]");
            return;
        }

        #endregion

        #region Preflight Checks

        if (!Repository.IsValid(Directory.GetCurrentDirectory()))
        {
            AnsiConsole.MarkupLine("[red]MBSS is not running inside a Git repository, aborting.[/]");
            return;
        }

        if (!File.Exists(".gitignore"))
        {
            AnsiConsole.MarkupLine("[red]Git repository does not have a .gitignore, aborting.[/]");
            AnsiConsole.MarkupLine("[red]It is absolutely necessary to ignore the bin/ and downloads/ directories![/]");
            return;
        }

        if (!File.Exists("bin/DepotDownloader.exe")) await GetDepotDownloader(client);
        if (!File.Exists("bin/GenericStripper.exe")) await GetGenericStripper(client);

        #endregion

        var downloadDir = new DirectoryInfo("downloads");
        var versionsDir = new DirectoryInfo("versions");

        if (!downloadDir.Exists) downloadDir.Create();
        if (!versionsDir.Exists) versionsDir.Create();

        foreach (var version in versions)
        {
            var downloadPath = Path.Combine(downloadDir.FullName, $"{version.Version}");
            var versionPath = Path.Combine(versionsDir.FullName, $"{version.Version}");

            if (Directory.Exists(versionPath)) continue;

            var depotDownloader = new Process
            {
                StartInfo =
                {
                    FileName = "bin/DepotDownloader.exe",
                    Arguments =
                        $"-app 620980 -depot 620981 -manifest \"{version.Manifest}\" -dir {downloadPath} -remember-password -username \"{Environment.GetEnvironmentVariable("STEAM_USERNAME")}\" -password \"{Environment.GetEnvironmentVariable("STEAM_PASSWORD")}\""
                }
            };

            depotDownloader.Start();
            await depotDownloader.WaitForExitAsync();

            var genericStripper = new Process
            {
                StartInfo =
                {
                    FileName = "bin/GenericStripper.exe",
                    Arguments = $"strip -m beatsaber -p \"{downloadPath}\" -o \"{versionPath}\""
                }
            };

            genericStripper.Start();
            await genericStripper.WaitForExitAsync();

            AnsiConsole.MarkupLine($"[green]Stripped {version.Version}![/]");

            using var repo = new Repository(Directory.GetCurrentDirectory());
            var author = new Signature(Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"),
                Environment.GetEnvironmentVariable("GIT_AUTHOR_EMAIL"), DateTimeOffset.Now);

            Commands.Stage(repo, versionPath);
            repo.Commit($"chore: v{version.Version}", author, author);

            Directory.Delete(downloadPath, true);
            var remote = repo.Network.Remotes["origin"];
            var options = new PushOptions
            {
                CredentialsProvider = (_, _, _) => new UsernamePasswordCredentials
                {
                    Username = Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"),
                    Password = Environment.GetEnvironmentVariable("GITHUB_TOKEN")
                }
            };

            if (remote != null)
                repo.Network.Push(remote, @"refs/heads/main", options);
        }
    }

    private static void InitConsole()
    {
        AnsiConsole.MarkupLine("[bold yellow]MBSS - Mass Beat Saber Stripper[/]");
        AnsiConsole.MarkupLine(
            "[green]This program will download and strip the Beat Saber versions listed in versions.json.[/]");
        AnsiConsole.MarkupLine(
            "[green]It will then commit and push the stripped versions to the main branch of the repository.[/]");
        AnsiConsole.MarkupLine(
            "[green]Ensure you are running MBSS inside the root of your desired versions repository![/]");
    }

    private static async Task SetupDotEnv()
    {
        var dotenv = await File.ReadAllLinesAsync(".env");
        foreach (var env in dotenv)
        {
            var split = env.Split('=', StringSplitOptions.RemoveEmptyEntries);
            if (split.Length != 2) continue;
            Environment.SetEnvironmentVariable(split[0], split[1]);
        }
    }

    private static async Task GetDepotDownloader(HttpClient client)
    {
        AnsiConsole.MarkupLine("[yellow]DepotDownloader.exe does not exist, downloading...[/]");

        var res = await client.GetAsync("https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest");
        if (res.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to get DepotDownloader release!");

        var latestRelease =
            JsonConvert.DeserializeObject<Dictionary<string, dynamic>>(res.Content.ReadAsStringAsync().Result);
        if (latestRelease == null) throw new Exception("Failed to parse DepotDownloader release!");

        var assets = latestRelease["assets"] as JArray;
        var asset = assets?.FirstOrDefault(x => x["name"]?.ToString().Contains("windows-x64") ?? false);
        if (asset == null) throw new Exception("Failed to find a DepotDownloader asset for this system!");

        var assetRes = client.GetAsync(asset["browser_download_url"]?.ToString()).Result;
        if (assetRes.StatusCode != HttpStatusCode.OK)
            throw new Exception("Failed to download DepotDownloader asset!");

        await using var assetStream = assetRes.Content.ReadAsStreamAsync().Result;
        using var archive = new ZipArchive(assetStream);
        archive.ExtractToDirectory(Path.Combine(Directory.GetCurrentDirectory(), "bin"));
    }

    private static async Task GetGenericStripper(HttpClient client)
    {
        AnsiConsole.MarkupLine("[yellow]GenericStripper.exe does not exist, downloading...[/]");

        var res = await client.GetAsync("https://api.github.com/repos/beat-forge/GenericStripper/releases/latest");
        if (res.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to get GenericStripper release!");

        var latestRelease =
            JsonConvert.DeserializeObject<Dictionary<string, dynamic>>(res.Content.ReadAsStringAsync().Result);
        if (latestRelease == null) throw new Exception("Failed to parse GenericStripper release!");

        var assets = latestRelease["assets"] as JArray;
        var asset = assets?.FirstOrDefault(x => x["name"]?.ToString().Contains("GenericStripper") ?? false);
        if (asset == null) throw new Exception("Failed to find a GenericStripper asset for this system!");

        var assetRes = client.GetAsync(asset["browser_download_url"]?.ToString()).Result;
        if (assetRes.StatusCode != HttpStatusCode.OK)
            throw new Exception("Failed to download GenericStripper asset!");

        await using var assetStream = assetRes.Content.ReadAsStreamAsync().Result;
        using var archive = new ZipArchive(assetStream);
        archive.ExtractToDirectory(Path.Combine(Directory.GetCurrentDirectory(), "bin"));
    }
}