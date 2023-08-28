using System.Diagnostics;
using System.IO.Compression;
using System.Net;
using Newtonsoft.Json;
using LibGit2Sharp;
using Newtonsoft.Json.Linq;

namespace MassBeatSaberStripper;

internal class BeatSaberVersion
{
    [JsonProperty("version")] public string Version { get; set; } = string.Empty;
    [JsonProperty("depot")] public string Depot { get; set; } = string.Empty;
}

internal abstract class Program
{
    public static async Task Main()
    {
        var versions = JsonConvert.DeserializeObject<List<BeatSaberVersion>>(await File.ReadAllTextAsync("versions.json"));
        if (versions == null) throw new Exception("Failed to parse versions.json!");
        
        if (!File.Exists("DepotDownloader.exe"))
        {
            Console.WriteLine("Downloading DepotDownloader...");
            var client = new HttpClient();
            client.DefaultRequestHeaders.Add("User-Agent", "MassBeatSaberStripper");
            
            var res = await client.GetAsync("https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest");
            if (res.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to get DepotDownloader release!");
            
            var latestRelease = JsonConvert.DeserializeObject<Dictionary<string, dynamic>>(res.Content.ReadAsStringAsync().Result);
            if (latestRelease == null) throw new Exception("Failed to parse DepotDownloader release!");
            
            var assets = latestRelease["assets"] as JArray;
            var asset = assets?.FirstOrDefault(x => x["name"]?.ToString().Contains("windows-x64") ?? false);
            if (asset == null) throw new Exception("Failed to find a DepotDownloader asset for this system!");
            
            var assetRes = client.GetAsync(asset["browser_download_url"]?.ToString()).Result;
            if (assetRes.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to download DepotDownloader asset!");

            await using var assetStream = assetRes.Content.ReadAsStreamAsync().Result;
            using var archive = new ZipArchive(assetStream);
            archive.ExtractToDirectory(Directory.GetCurrentDirectory());
        }
        
        if (!File.Exists("GenericStripper.exe"))
        {
            Console.WriteLine("Downloading GenericStripper...");
            var client = new HttpClient();
            client.DefaultRequestHeaders.Add("User-Agent", "MassBeatSaberStripper");
            
            var res = await client.GetAsync("https://api.github.com/repos/beat-forge/GenericStripper/releases/latest");
            if (res.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to get GenericStripper release!");
            
            var latestRelease = JsonConvert.DeserializeObject<Dictionary<string, dynamic>>(res.Content.ReadAsStringAsync().Result);
            if (latestRelease == null) throw new Exception("Failed to parse GenericStripper release!");
            
            var assets = latestRelease["assets"] as JArray;
            var asset = assets?.FirstOrDefault(x => x["name"]?.ToString().Contains("GenericStripper") ?? false);
            if (asset == null) throw new Exception("Failed to find a GenericStripper asset for this system!");
            
            var assetRes = client.GetAsync(asset["browser_download_url"]?.ToString()).Result;
            if (assetRes.StatusCode != HttpStatusCode.OK) throw new Exception("Failed to download GenericStripper asset!");

            await using var assetStream = assetRes.Content.ReadAsStreamAsync().Result;
            using var archive = new ZipArchive(assetStream);
            // todo: fix zip structure so this isn't needed
            foreach (var entry in archive.Entries)
            {
                if (!entry.FullName.Contains("net7.0")) continue;
                try { entry.ExtractToFile(Path.Combine(Directory.GetCurrentDirectory(), entry.Name)); }
                catch
                {
                    // ignored
                }
            }
        }

        var downloadDir = new DirectoryInfo("downloads");
        var strippedDir = new DirectoryInfo("versions");
        
        if (!Directory.Exists(downloadDir.FullName)) Directory.CreateDirectory(downloadDir.FullName);
        if (!Directory.Exists(strippedDir.FullName)) Directory.CreateDirectory(strippedDir.FullName);
        
        if (!Repository.IsValid(Directory.GetCurrentDirectory()))
        {
            Repository.Init(Directory.GetCurrentDirectory());
        }
        
        foreach (var version in versions)
        {
            var versionDownloadDir = Path.Combine(downloadDir.FullName, $"{version.Version}");
            var versionStrippedPath = Path.Combine(strippedDir.FullName, $"{version.Version}");

            if (Directory.Exists(versionStrippedPath))
            {
                Console.WriteLine($"Skipping {version.Version}, already exists!");
                continue;
            }

            Console.WriteLine($"Downloading {version.Version}...");
            var depotDownloader = new Process
            {
                StartInfo =
                {
                    FileName = "DepotDownloader.exe",
                    Arguments =
                        $"-app 620980 -depot 620981 -manifest \"{version.Depot}\" -dir {versionDownloadDir} -remember-password -username \"{Environment.GetEnvironmentVariable("STEAM_USERNAME")}\" -password \"{Environment.GetEnvironmentVariable("STEAM_PASSWORD")}\""
                }
            };

            depotDownloader.Start();
            await depotDownloader.WaitForExitAsync();

            // strip the game 
            var genericStripper = new Process
            {
                StartInfo =
                {
                    FileName = "GenericStripper.exe",
                    Arguments = $"strip -m beatsaber -p \"{versionDownloadDir}\" -o \"{versionStrippedPath}\""
                }
            };
            
            genericStripper.Start();
            await genericStripper.WaitForExitAsync();
            
            // add the changes
            using var repo = new Repository(Directory.GetCurrentDirectory());
            Commands.Stage(repo, versionStrippedPath);
            
            // commit the changes
            var author = new Signature(Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"), Environment.GetEnvironmentVariable("GIT_AUTHOR_EMAIL"), DateTimeOffset.Now);
            repo.Commit($"Stripped {version.Version}", author, author);
            
            // delete the download directory
            Directory.Delete(versionDownloadDir, true);
            Console.WriteLine($"Stripped {version.Version}!");
            
            // push the changes
            var remote = repo.Network.Remotes["origin"];
            var options = new PushOptions
            {
                CredentialsProvider = (_, _, _) => new UsernamePasswordCredentials
                {
                    Username = Environment.GetEnvironmentVariable("GIT_AUTHOR_NAME"),
                    Password = Environment.GetEnvironmentVariable("GITHUB_TOKEN")
                }
            };
            
            repo.Network.Push(remote, @"refs/heads/main", options);
        }
    }
}