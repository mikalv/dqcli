use crate::types::Availability;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const WHOIS_PORT: u16 = 43;

pub async fn check_whois(domain: &str, timeout: Duration) -> Availability {
    let tld = match domain.rsplit('.').next() {
        Some(t) => t.to_lowercase(),
        None => return Availability::Unknown { reason: "Invalid domain".to_string() },
    };
    
    let whois_server = match tld.as_str() {
        "com" | "net" => "whois.verisign-grs.com",
        "org" => "whois.pir.org",
        "io" => "whois.nic.io",
        "dev" | "app" => "whois.nic.google",
        "ai" => "whois.nic.ai",
        "co" => "whois.nic.co",
        "me" => "whois.nic.me",
        _ => return Availability::Unknown { reason: format!("No WHOIS server for .{}", tld) },
    };

    let result = tokio::time::timeout(timeout, async {
        let mut stream = TcpStream::connect((whois_server, WHOIS_PORT)).await?;
        stream.write_all(format!("{}\r\n", domain).as_bytes()).await?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        
        Ok::<_, std::io::Error>(response)
    }).await;

    match result {
        Ok(Ok(response)) => {
            let lower = response.to_lowercase();
            if lower.contains("no match") 
                || lower.contains("not found") 
                || lower.contains("no data found")
                || lower.contains("no entries found")
            {
                Availability::Available
            } else if lower.contains("domain name:") || lower.contains("registrar:") {
                Availability::Taken
            } else {
                Availability::Unknown { reason: "Ambiguous WHOIS response".to_string() }
            }
        }
        Ok(Err(e)) => Availability::Unknown { reason: format!("WHOIS error: {}", e) },
        Err(_) => Availability::Unknown { reason: "WHOIS timeout".to_string() },
    }
}
