//! Public release checks are independent of UU login and device sessions.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
mod settings;

// A fresh repository must never offer upgrades from the former product.
// Release CI injects its own owner/repository; local builds stay offline here.
fn repository() -> Option<&'static str> {
    option_env!("OMARCHYUU_GITHUB_REPOSITORY").filter(|value| valid_repository(value))
}
fn valid_repository(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}
pub(super) fn configured() -> bool {
    repository().is_some()
}
const RETRY_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(super) enum State {
    Checking,
    Current,
    Ahead,
    Available {
        version: String,
        url: String,
        notes: String,
        published: Option<String>,
        prerelease: bool,
    },
    NoRelease,
    Failed(String),
}

pub(super) struct UpdateCheck {
    pub state: State,
    pub dialog_open: bool,
    notified_version: Option<String>,
    pending: Option<Receiver<Result<State>>>,
    cancel: Option<oneshot::Sender<()>>,
    retry_at: Instant,
    settings: settings::Settings,
    pub settings_error: Option<String>,
}

impl UpdateCheck {
    pub fn start(ctx: &egui::Context) -> Self {
        let (settings, settings_error) = match settings::Settings::load() {
            Ok(settings) => (settings, None),
            Err(error) => (Default::default(), Some(format!("{error:#}"))),
        };
        let mut check = Self {
            state: State::Checking,
            dialog_open: false,
            notified_version: None,
            pending: None,
            cancel: None,
            retry_at: Instant::now(),
            settings,
            settings_error,
        };
        check.request(ctx);
        check
    }
    pub fn allow_prerelease(&self) -> bool {
        self.settings.allow_prerelease
    }
    pub fn release_label(&self) -> &'static str {
        if self.allow_prerelease() {
            "可用版本"
        } else {
            "正式版"
        }
    }
    pub fn set_allow_prerelease(&mut self, allow: bool, ctx: &egui::Context) -> Result<()> {
        if allow == self.allow_prerelease() {
            return Ok(());
        }
        let next = settings::Settings {
            allow_prerelease: allow,
        };
        if let Err(error) = next.save() {
            self.settings_error = Some(format!("{error:#}"));
            return Err(error);
        }
        self.change_channel(next);
        self.request(ctx);
        Ok(())
    }
    fn change_channel(&mut self, next: settings::Settings) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        // Drop the old mailbox before a replacement can publish its result.
        self.pending = None;
        self.settings = next;
        self.settings_error = None;
        self.dialog_open = false;
        self.notified_version = None;
        self.state = State::Checking;
        self.retry_at = Instant::now();
    }

    pub fn request(&mut self, ctx: &egui::Context) {
        if !configured() {
            self.state = State::NoRelease;
            return;
        }
        if self.pending.is_some() || Instant::now() < self.retry_at {
            return;
        }
        self.retry_at = Instant::now() + RETRY_INTERVAL;
        let (tx, rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let ctx = ctx.clone();
        let allow_prerelease = self.allow_prerelease();
        let worker = std::thread::Builder::new()
            .name("release-check".into())
            .spawn(move || {
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        let result = runtime.block_on(async {
                            tokio::select! {
                                biased;
                                _ = cancel_rx => None,
                                result = tokio::time::timeout(Duration::from_secs(25), latest(allow_prerelease)) =>
                                    Some(result.context("更新检查超时").and_then(|r|r)),
                            }
                        });
                        runtime.shutdown_background();
                        result
                    }
                    Err(error) => Some(Err(error).context("无法启动更新检查")),
                };
                if let Some(result) = result {
                    let _ = tx.send(result);
                    ctx.request_repaint();
                }
            });
        match worker {
            Ok(_) => {
                self.pending = Some(rx);
                self.cancel = Some(cancel_tx);
                self.state = State::Checking;
            }
            Err(error) => self.state = State::Failed(format!("无法启动更新检查：{error}")),
        }
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("更新检查未完成")),
        };
        self.pending = None;
        self.cancel = None;
        self.state = result.unwrap_or_else(|error| {
            tracing::warn!(error = %format!("{error:#}"), "release check failed");
            State::Failed(format!("{error:#}"))
        });
        if let State::Available { version, .. } = &self.state
            && self.notified_version.as_ref() != Some(version)
        {
            self.notified_version = Some(version.clone());
            self.dialog_open = true;
        }
    }

    pub fn retry_wait(&self) -> u64 {
        self.retry_at
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .ceil() as u64
    }
}

impl Drop for UpdateCheck {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    published_at: Option<String>,
}

async fn latest(allow_prerelease: bool) -> Result<State> {
    let repository = repository().context("尚未配置新仓库的更新源")?;
    let releases_url = format!("https://github.com/{repository}/releases");
    let api = format!("https://api.github.com/repos/{repository}/releases");
    let client = crate::transport::http_client()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("omarchyuu/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("无法创建更新检查连接")?;
    if !allow_prerelease {
        return match fetch(&client, &format!("{api}/latest"), 512 * 1024).await? {
            Some(body) => release_state(&body, env!("CARGO_PKG_VERSION"), &releases_url),
            None => Ok(State::NoRelease),
        };
    }
    let mut releases = Vec::new();
    let mut total_bytes = 0usize;
    for page in 1..=10 {
        let url = format!("{api}?per_page=100&page={page}");
        let Some(body) = fetch(&client, &url, 2 * 1024 * 1024).await? else {
            if page == 1 {
                return Ok(State::NoRelease);
            }
            bail!("GitHub 版本列表读取不完整，请重试");
        };
        total_bytes += body.len();
        ensure!(total_bytes <= 8 * 1024 * 1024, "GitHub 版本列表过大");
        let batch: Vec<Release> =
            serde_json::from_slice(&body).context("无法解析 GitHub 版本列表")?;
        let complete = batch.len() < 100;
        releases.extend(batch);
        if complete {
            return select_release(releases, env!("CARGO_PKG_VERSION"), true, &releases_url);
        }
    }
    bail!("GitHub 版本列表过长，未完成检查")
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    maximum_bytes: usize,
) -> Result<Option<Vec<u8>>> {
    let mut response = crate::transport::http::send(
        crate::transport::http::Operation::Update,
        client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10"),
    )
    .await
    .context("无法连接 GitHub，请检查网络后重试")?;
    match response.status() {
        reqwest::StatusCode::NOT_FOUND => return Ok(None),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::TOO_MANY_REQUESTS => {
            bail!("GitHub 暂时限制请求，请稍后重试")
        }
        _ => {
            response
                .error_for_status_ref()
                .context("GitHub 更新检查失败")?;
            ensure!(
                response.status().is_success(),
                "GitHub 更新接口返回意外 HTTP 状态：{}",
                response.status()
            );
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.context("读取版本信息失败")? {
        ensure!(
            body.len() + chunk.len() <= maximum_bytes,
            "GitHub 版本信息过大"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

/// Linux packaging revisions are not part of the application's semver channel.
fn release_version(tag: &str) -> Result<semver::Version, semver::Error> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let version = match tag.rsplit_once("-linux.") {
        Some((version, revision))
            if !revision.is_empty() && revision.bytes().all(|c| c.is_ascii_digit()) =>
        {
            version
        }
        _ => tag,
    };
    semver::Version::parse(version)
}

fn select_release(
    releases: Vec<Release>,
    current: &str,
    allow_prerelease: bool,
    releases_url: &str,
) -> Result<State> {
    let selected = releases
        .into_iter()
        .filter_map(|release| {
            if release.draft || release.published_at.is_none() {
                return None;
            }
            let version = release_version(&release.tag_name).ok()?;
            if !allow_prerelease && (release.prerelease || !version.pre.is_empty()) {
                return None;
            }
            Some((version, release))
        })
        .max_by(|(a, ra), (b, rb)| {
            a.cmp_precedence(b)
                .then_with(|| rb.prerelease.cmp(&ra.prerelease))
        });
    match selected {
        Some((version, release)) => state_for_release(release, version, current, releases_url),
        None => Ok(State::NoRelease),
    }
}

fn release_state(body: &[u8], current: &str, releases_url: &str) -> Result<State> {
    let release: Release = serde_json::from_slice(body).context("无法解析 GitHub 版本信息")?;
    ensure!(
        !release.draft && !release.prerelease,
        "GitHub 未返回正式版本"
    );
    let version = release_version(&release.tag_name).context("GitHub 正式版本号格式不正确")?;
    ensure!(version.pre.is_empty(), "GitHub 最新版本不是正式版本");
    state_for_release(release, version, current, releases_url)
}

fn state_for_release(
    release: Release,
    version: semver::Version,
    current: &str,
    releases_url: &str,
) -> Result<State> {
    let current = semver::Version::parse(current).context("当前程序版本号格式不正确")?;
    match version.cmp_precedence(&current) {
        std::cmp::Ordering::Equal => Ok(State::Current),
        std::cmp::Ordering::Less => Ok(State::Ahead),
        std::cmp::Ordering::Greater => {
            let mut url = url::Url::parse(releases_url)?;
            url.path_segments_mut()
                .expect("GitHub base URL")
                .push("tag")
                .push(&release.tag_name);
            Ok(State::Available {
                prerelease: release.prerelease || !version.pre.is_empty(),
                version: version.to_string(),
                url: url.to_string(),
                notes: release.body.unwrap_or_default(),
                published: release.published_at.and_then(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .ok()
                        .map(|date| date.format("%Y-%m-%d").to_string())
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_repository_does_not_start_a_legacy_update_request() {
        assert!(valid_repository("example/omarchyuu"));
        for invalid in [
            "",
            "old",
            "a/b/c",
            "../omarchyuu",
            "https://github.com/a/b",
            "a/b?x=1",
        ] {
            assert!(!valid_repository(invalid));
        }
        if !configured() {
            let check = UpdateCheck::start(&egui::Context::default());
            assert!(matches!(check.state, State::NoRelease));
            assert!(check.pending.is_none());
        }
    }
    const RELEASES: &str = "https://github.com/example/omarchyuu/releases";

    #[test]
    fn linux_packaging_revision_preserves_the_release_channel() {
        let stable = br#"{"tag_name":"v1.0.2-linux.3","draft":false,"prerelease":false}"#;
        assert!(matches!(
            release_state(stable, "1.0.2", RELEASES).unwrap(),
            State::Current
        ));
        assert!(matches!(
            release_state(stable, "1.0.1", RELEASES).unwrap(),
            State::Available {
                prerelease: false,
                ..
            }
        ));
        let beta = release_version("v1.0.3-beta.5-linux.4").unwrap();
        assert_eq!(beta.pre.as_str(), "beta.5");
        assert!(
            release_state(
                br#"{"tag_name":"v1.0.3-beta.5-linux.4","draft":false,"prerelease":false}"#,
                "1.0.2",
                RELEASES
            )
            .is_err()
        );
    }

    #[test]
    fn update_links_follow_the_configured_repository() {
        let state = release_state(
            br#"{"tag_name":"v2.0.0-linux.1","draft":false,"prerelease":false}"#,
            "0.1.0",
            RELEASES,
        )
        .unwrap();
        let State::Available { url, .. } = state else {
            panic!("expected update")
        };
        assert_eq!(
            url,
            "https://github.com/example/omarchyuu/releases/tag/v2.0.0-linux.1"
        );
    }
}
