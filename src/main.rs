use reqwest::Client;
use scraper::{Html, Selector};
use std::collections::{HashSet, VecDeque};
use std::env;
use std::error::Error;
use std::fs::File;
use url::Url;

use csv::Writer;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Product {
    url: String,
    name: String,
    price: String,
}

#[tokio::main]
async fn main() {
    // CLI: cargo run -- <start_url> [max_pages]
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: cargo run -- <start_url> [max_pages]");
        return;
    }

    let start_url_str = &args[1];
    let max_pages: usize = if args.len() >= 3 {
        args[2].parse().unwrap_or(20)
    } else {
        20
    };

    let start_url = match Url::parse(start_url_str) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Invalid start URL: {e}");
            return;
        }
    };

    println!("Starting crawl at: {}", start_url);
    println!("Max pages: {}", max_pages);

    if let Err(e) = crawl_and_extract(start_url, max_pages).await {
        eprintln!("Crawl failed: {e}");
    }
}

async fn crawl_and_extract(start_url: Url, max_pages: usize) -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    let link_selector = Selector::parse("a").unwrap();

    let mut queue: VecDeque<Url> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    let start_domain = start_url.domain().map(|d| d.to_string());

    // CSV writer – will write to products.csv in project root
    let file = File::create("products.csv")?;
    let mut writer = Writer::from_writer(file);

    // CSV header
    writer.write_record(&["url", "name", "price"])?;

    queue.push_back(start_url);

    while let Some(url) = queue.pop_front() {
        if visited.len() >= max_pages {
            println!("\nReached max pages limit ({}) – stopping crawl.", max_pages);
            break;
        }

        let url_str = url.as_str().to_string();
        if visited.contains(&url_str) {
            continue;
        }

        println!("\n=== Fetching ({}/{}) ===", visited.len() + 1, max_pages);
        println!("{url_str}");

        visited.insert(url_str.clone());

        let body = match client.get(url.clone()).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("Failed to read body for {}: {}", url, e);
                    continue;
                }
            },
            Err(e) => {
                eprintln!("Request failed for {}: {}", url, e);
                continue;
            }
        };

        // Extract product data from this page
        extract_products(&body, &url, &mut writer)?;

        // Normal link discovery to keep crawling within same domain
        let document = Html::parse_document(&body);

        for element in document.select(&link_selector) {
            if let Some(href) = element.value().attr("href") {
                if let Ok(next_url) = url.join(href) {
                    if let Some(ref domain) = start_domain {
                        if next_url
                            .domain()
                            .map(|d| d != domain)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                    }

                    let next_str = next_url.as_str().to_string();
                    if !visited.contains(&next_str) {
                        queue.push_back(next_url);
                    }
                }
            }
        }
    }

    writer.flush()?;
    println!("\nCrawl complete. Total pages visited: {}", visited.len());
    println!("Saved extracted products to products.csv");

    Ok(())
}

/// Try to extract product name + price from a Walmart-like search result page.
/// NOTE: selectors may need adjustment if Walmart changes their HTML.
fn extract_products(
    html: &str,
    page_url: &Url,
    writer: &mut Writer<File>,
) -> Result<(), Box<dyn Error>> {
    let document = Html::parse_document(html);

    // Each product tile – this is a best-effort selector.
    // You can refine this by inspecting Walmart's HTML with browser dev tools.
    let product_selector =
        Selector::parse("div[data-item-id], div[data-automation-id='productTile']").unwrap();

    // Name and price selectors (fallback to common patterns)
    let name_selector = Selector::parse(
        "[data-automation-id='product-title'], a[aria-label], div[data-automation-id='product-title-link']",
    )
    .unwrap();
    let price_selector = Selector::parse(
        "[data-automation-id='product-price'], span[aria-hidden='true'], div.price-main span",
    )
    .unwrap();

    let link_selector = Selector::parse("a").unwrap();

    for product in document.select(&product_selector) {
        // Name
        let name = product
            .select(&name_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Price (this may include currency symbol)
        let price = product
            .select(&price_selector)
            .next()
            .map(|e| e.text().collect::<String>())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if name.is_empty() || price.is_empty() {
            continue;
        }

        // Product URL (first link inside the tile)
        let product_url = product
            .select(&link_selector)
            .next()
            .and_then(|a| a.value().attr("href"))
            .and_then(|href| page_url.join(href).ok())
            .map(|u| u.to_string())
            .unwrap_or_else(|| page_url.to_string());

        writer.serialize(Product {
            url: product_url,
            name,
            price,
        })?;
    }

    Ok(())
}
