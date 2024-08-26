use crate::arch::Arch;
use regex::Regex;
use reqwest::Url;
use scraper::{Html, Selector};
use semver::Version;
use std::collections::BTreeMap;

pub async fn get_docker_releases(arch: Arch) -> anyhow::Result<BTreeMap<Version, Url>> {
    let index_url = Url::parse(&format!("https://download.docker.com/linux/static/stable/{}/", arch.as_uname_m())).unwrap();
    let html = reqwest::get(index_url.clone()).await?.error_for_status()?.text().await?;
    let html = Html::parse_document(&html);
    let releases = html
        .select(&Selector::parse("a[href]").unwrap())
        .into_iter()
        .map(|a| a.attr("href").unwrap())
        .flat_map(|href| Regex::new(r"^docker-(\d+\.\d+\.\d+)\.tgz$").unwrap().captures(href))
        .flat_map(|captures| {
            let url = captures.get(0).unwrap().as_str();
            let url = index_url.join(url);

            let version = captures.get(1).unwrap().as_str();
            let version = Version::parse(version);

            match (url, version) {
                (Ok(url), Ok(version)) => Some((version, url)),
                _ => None,
            }
        })
        .collect();

    Ok(releases)
}
