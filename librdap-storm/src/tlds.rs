use reqwest::Client;
use thiserror::Error;

const IANA_TLD_LIST_URL: &str = "https://data.iana.org/TLD/tlds-alpha-by-domain.txt";

#[derive(Debug, Error)]
pub enum TldError {
    #[error("Failed to fetch TLD list: {0}")]
    FetchError(#[from] reqwest::Error),
}

pub async fn fetch_iana_tlds(client: &Client) -> Result<Vec<String>, TldError> {
    let response = client.get(IANA_TLD_LIST_URL).send().await?.text().await?;
    
    let tlds: Vec<String> = response
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.trim().to_lowercase())
        .filter(|tld| !tld.starts_with("xn--"))
        .collect();
    
    Ok(tlds)
}

pub fn expand_tlds<'a>(name: &'a str, tlds: &'a [String]) -> impl Iterator<Item = String> + 'a {
    tlds.iter().map(move |tld| format!("{}.{}", name, tld))
}
