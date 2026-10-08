use std::fs;

use zed_extension_api::{self as zed, make_file_executable, Result};

use crate::language_servers::util;

pub struct KotlinLSP {
    cached_binary_path: Option<String>,
}

impl KotlinLSP {
    pub const LANGUAGE_SERVER_ID: &'static str = "kotlin-lsp";

    pub fn new() -> Self {
        KotlinLSP {
            cached_binary_path: None,
        }
    }

    pub fn language_server_binary_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
    ) -> Result<String> {
        if let Some(path) = self.cached_binary_path.as_ref() {
            return Ok(path.clone());
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let binary_path = download_from_teamcity(language_server_id)?;

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

/// Fetch the kotlin-lsp release notes.
fn fetch_release_notes() -> Result<String> {
    let url = "https://raw.githubusercontent.com/Kotlin/kotlin-lsp/refs/heads/main/RELEASES.md"
        .to_string();
    let result = zed::http_client::fetch(&zed::http_client::HttpRequest {
        method: zed::http_client::HttpMethod::Get,
        url,
        headers: vec![],
        body: None,
        redirect_policy: zed::http_client::RedirectPolicy::NoFollow,
    })?;
    String::from_utf8(result.body).map_err(|_| "RELEASES.md is not valid UTF-8".to_owned())
}

/// Is `url` a download link for the standalone kotlin-server archive?
///
/// The URL must be `https`, and its file name must be
/// `kotlin-server-{version}{arch_suffix}{file_ending}`, where the version
/// consists of digits and dots.
fn is_download_link(url: &str, arch_suffix: &str, file_ending: &str) -> bool {
    let expected_suffix = format!("{arch_suffix}{file_ending}");
    url.strip_prefix("https://")
        .and_then(|url| url.rsplit_once('/'))
        .map(|(_, file_name)| file_name)
        .and_then(|file_name| file_name.strip_prefix("kotlin-server-"))
        .and_then(|file_name| file_name.strip_suffix(&expected_suffix))
        .is_some_and(|version| {
            !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
}

/// Extract the version from a standalone kotlin-server archive download link
/// (see [`is_download_link`]). Yields `None` for URLs that are not download links.
fn kotlin_server_version_from_url<'a>(
    url: &'a str,
    arch_suffix: &str,
    file_ending: &str,
) -> Option<&'a str> {
    if !is_download_link(url, arch_suffix, file_ending) {
        return None;
    }
    let file_name = url.rsplit_once('/')?.1;
    file_name
        .strip_prefix("kotlin-server-")?
        .strip_suffix(&format!("{arch_suffix}{file_ending}"))
}

/// Find the standalone kotlin-server archive download URL in the release notes.
/// The notes list releases newest-first, so the first link matching our
/// constraints is the latest one.
fn find_download_url<'a>(
    release_notes: &'a str,
    arch_suffix: &str,
    file_ending: &str,
) -> Option<&'a str> {
    release_notes.lines().find_map(|line| {
        let mut rest = line;
        while let Some(url_start) = rest.find("https://") {
            let url = &rest[url_start..];
            let url_end = url
                .find(|c: char| c == ')' || c.is_whitespace())
                .unwrap_or(url.len());
            let url = &url[..url_end];
            if is_download_link(url, arch_suffix, file_ending) {
                return Some(url);
            }
            rest = &rest[url_start + url_end..];
        }
        None
    })
}

fn download_from_teamcity(language_server_id: &zed::LanguageServerId) -> Result<String> {
    let release_notes = fetch_release_notes()?;
    let (os, arch) = zed_extension_api::current_platform();

    let arch_suffix = match arch {
        zed::Architecture::X8664 => "",
        zed::Architecture::Aarch64 => "-aarch64",
        _ => {
            return Err("Platform X86 is not supported by the Kotlin language server.".to_string())
        }
    };

    let file_ending = match os {
        zed::Os::Mac => ".sit",
        zed::Os::Linux => ".tar.gz",
        zed::Os::Windows => ".win.zip",
    };

    let url = find_download_url(&release_notes, arch_suffix, file_ending).ok_or_else(|| {
        format!(
            "Found no https download link for kotlin-server-<version>{arch_suffix}{file_ending} in RELEASES.md"
        )
    })?;
    let version = kotlin_server_version_from_url(url, arch_suffix, file_ending)
        .ok_or_else(|| format!("Found no version in the download link {url}"))?;

    zed::set_language_server_installation_status(
        language_server_id,
        &zed::LanguageServerInstallationStatus::Downloading,
    );

    let extension_dir = format!(
        "{server_id}-{version}",
        server_id = KotlinLSP::LANGUAGE_SERVER_ID
    );

    let target_dir = match os {
        zed::Os::Windows => extension_dir.clone(),
        _ => format!("{extension_dir}/kotlin-server-{version}"),
    };

    let binary_path = format!(
        "{target_dir}/bin/intellij-server{exe_suffix}",
        exe_suffix = match os {
            zed::Os::Windows => ".exe",
            _ => "",
        }
    );

    if !fs::metadata(&extension_dir).is_ok_and(|metadata| metadata.is_dir()) {
        let downloaded_file_type = match os {
            // We don't ask questions as to why `sit` == `zip`. Let JetBrains keep their secrets there
            zed::Os::Windows | zed::Os::Mac => zed_extension_api::DownloadedFileType::Zip,
            zed::Os::Linux => zed_extension_api::DownloadedFileType::GzipTar,
        };

        zed::download_file(url, &extension_dir, downloaded_file_type)?;
        make_file_executable(&binary_path)?;
        util::remove_outdated_versions(KotlinLSP::LANGUAGE_SERVER_ID, &extension_dir)?;
    }

    Ok(binary_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed-down stand-in for the real RELEASES.md: a fake host and path,
    /// VS Code `.vsix` links using the same "Download for …" labels as the
    /// standalone archive, and an older release below the newest one.
    const RELEASE_NOTES: &str = r#"
### Kotlin LSP and VSC extension releases

### v263.6379.0
- :test_tube: "Kotlin by JetBrains" extension v0.0.13 for VS Code

  The extension is also available on the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=JetBrains.kotlin-server).
    * [Download for macOS-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-mac-amd64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-mac-amd64.vsix.sha256)
    * [Download for macOS-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-mac-aarch64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-mac-aarch64.vsix.sha256)
    * [Download for Linux-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-linux-amd64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-linux-amd64.vsix.sha256)
    * [Download for Linux-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-linux-aarch64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-linux-aarch64.vsix.sha256)
    * [Download for Windows-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-win-amd64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-win-amd64.vsix.sha256)
    * [Download for Windows-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-win-aarch64.vsix)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-0.0.13-win-aarch64.vsix.sha256)

- :card_index_dividers: **Standalone Kotlin LSP Archive**

  Standalone Kotlin Language Server version for editors other than VS Code.
    * [Download for macOS-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.sit)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.sit.sha256)
    * [Download for macOS-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.sit)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.sit.sha256)
    * [Download for Linux-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.tar.gz)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.tar.gz.sha256)
    * [Download for Linux-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.tar.gz)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.tar.gz.sha256)
    * [Download for Windows-x64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.win.zip)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.win.zip.sha256)
    * [Download for Windows-arm64](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.win.zip)&nbsp;&nbsp;|&nbsp;&nbsp;[SHA-256 checksum](https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.win.zip.sha256)

### v262.8190.0
- :card_index_dividers: **Standalone Kotlin LSP Archive**

  Older release on a different host.
    * [Download for macOS-x64](https://dl.example.org/old/kotlin-server-262.8190.0.sit)
    * [Download for macOS-arm64](https://dl.example.org/old/kotlin-server-262.8190.0-aarch64.sit)
    * [Download for Linux-x64](https://dl.example.org/old/kotlin-server-262.8190.0.tar.gz)
    * [Download for Linux-arm64](https://dl.example.org/old/kotlin-server-262.8190.0-aarch64.tar.gz)
    * [Download for Windows-x64](https://dl.example.org/old/kotlin-server-262.8190.0.win.zip)
    * [Download for Windows-arm64](https://dl.example.org/old/kotlin-server-262.8190.0-aarch64.win.zip)
"#;

    #[test]
    fn finds_the_latest_standalone_download_link_for_every_platform() {
        let cases = [
            (
                "",
                ".sit",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.sit",
            ),
            (
                "-aarch64",
                ".sit",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.sit",
            ),
            (
                "",
                ".tar.gz",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.tar.gz",
            ),
            (
                "-aarch64",
                ".tar.gz",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.tar.gz",
            ),
            (
                "",
                ".win.zip",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0.win.zip",
            ),
            (
                "-aarch64",
                ".win.zip",
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.win.zip",
            ),
        ];

        for (arch_suffix, file_ending, expected_url) in cases {
            let url = find_download_url(RELEASE_NOTES, arch_suffix, file_ending)
                .unwrap_or_else(|| panic!("no download link for {arch_suffix}{file_ending}"));
            assert_eq!(
                url, expected_url,
                "arch_suffix: {arch_suffix:?}, file_ending: {file_ending:?}"
            );
            // The matcher and the version extractor share `is_download_link`,
            // but the extractor slices out the version on its own; they must agree.
            assert_eq!(
                kotlin_server_version_from_url(url, arch_suffix, file_ending),
                Some("263.6379.0"),
                "no version extracted from {url}"
            );
        }
    }

    #[test]
    fn skips_plain_http_and_non_url_text() {
        let notes = "\
plain text mention of kotlin-server-263.6379.0.sit
* [not tls](http://downloads.example.com/kotlin-server-263.6379.0.sit)";
        assert_eq!(find_download_url(notes, "", ".sit"), None);
    }

    #[test]
    fn does_not_fall_back_to_another_architectures_asset() {
        let notes =
            "* [aarch64 only](https://downloads.example.com/kotlin-server-263.6379.0-aarch64.sit)";
        assert_eq!(find_download_url(notes, "", ".sit"), None);
    }

    #[test]
    fn extracts_the_version_from_a_matching_url() {
        assert_eq!(
            kotlin_server_version_from_url(
                "https://downloads.example.com/new-path/263.6379.0/kotlin-server-263.6379.0-aarch64.sit",
                "-aarch64",
                ".sit",
            ),
            Some("263.6379.0")
        );
        assert_eq!(
            kotlin_server_version_from_url(
                "https://dl.example.org/old/kotlin-server-262.8190.0.win.zip",
                "",
                ".win.zip",
            ),
            Some("262.8190.0")
        );
    }

    #[test]
    fn rejects_urls_that_are_not_standalone_server_archives() {
        for url in [
            "https://downloads.example.com/kotlin-server-0.0.13-mac-amd64.vsix",
            "https://downloads.example.com/kotlin-server-263.6379.0.sit.sha256",
            "http://downloads.example.com/kotlin-server-263.6379.0.sit",
            "https://downloads.example.com/kotlin-server-not-a-version.sit",
            "https://downloads.example.com/kotlin-server-.sit",
        ] {
            assert_eq!(
                kotlin_server_version_from_url(url, "", ".sit"),
                None,
                "url: {url}"
            );
        }
    }
}
