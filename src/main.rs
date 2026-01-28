/*
 *  EPLMetadataGenerator - Metadata generator for ElyPrismLauncher
 *  Copyright (C) 2025 Octol1ttle <l1ttleofficial@outlook.com>
 *
 *  This program is free software: you can redistribute it and/or modify
 *  it under the terms of the GNU General Public License as published by
 *  the Free Software Foundation, either version 3 of the License, or
 *  (at your option) any later version.
 *
 *  This program is distributed in the hope that it will be useful,
 *  but WITHOUT ANY WARRANTY; without even the implied warranty of
 *  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *  GNU General Public License for more details.
 *
 *  You should have received a copy of the GNU General Public License
 *  along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use reqwest::Error;
use roxmltree::{Document, Node};
use sha1::{Digest, Sha1};
use std::collections::HashMap;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 5 {
        eprintln!("Not enough arguments, expected 4");
        eprintln!("1) URL to Maven metadata XML");
        eprintln!("2) Ely.by Authlib download URL format string ({{}} will be replaced with the version");
        eprintln!("3) authlib-injector download URL");
        eprintln!("4) Output file name");
        return
    }

    let _program_name = &args[0];
    let metadata_url = &args[1];
    let authlib_download_url_format = &args[2];
    let injector_download_url = &args[3];
    let output_file = &args[4];

    let http_client = reqwest::Client::new();

    let injector_download = http_client.get(injector_download_url).send();

    let metadata = http_client.get(metadata_url).send().await
        .expect("Couldn't download Maven metadata")
        .text().await
        .expect("Couldn't get text from metadata response");
    let metadata_doc = Document::parse(&metadata).expect("Couldn't parse Maven metadata");

    let metadata_versions: Vec<Node> = metadata_doc.descendants().find(|n| { n.has_tag_name("metadata") }).unwrap()
        .children().find(|n| n.has_tag_name("versioning")).unwrap()
        .children().find(|n| n.has_tag_name("versions")).unwrap()
        .children().filter(|n| n.has_tag_name("version")).collect();

    let mut authlib_versions_to_full_versions: HashMap<String, &str> = HashMap::new();
    for version in metadata_versions {
        let full_version = version.text().unwrap();
        let authlib_version = full_version.split('-').collect::<Vec<_>>()[0];

        if let Some((_, existing)) = authlib_versions_to_full_versions.get_key_value(authlib_version) {
            let existing_patch_number: i32 = existing.split('.').collect::<Vec<_>>().last().unwrap().parse().unwrap();
            let new_patch_number: i32 = full_version.split('.').collect::<Vec<_>>().last().unwrap().parse().unwrap();

            if new_patch_number > existing_patch_number {
                authlib_versions_to_full_versions.remove(authlib_version);
            } else {
                continue
            }
        }

        authlib_versions_to_full_versions.insert(authlib_version.to_string(), full_version);
    }

    let mut authlib_versions: Vec<&String> = authlib_versions_to_full_versions.keys().collect();
    authlib_versions.sort_by_key(|x| {
        let version_numbers: Vec<&str> = x.split('.').collect();
        let mut score = 0;

        score += 1_000_000 * version_numbers[0].parse::<i32>().unwrap();
        score += 1_000 * version_numbers[1].parse::<i32>().unwrap();
        if version_numbers.len() > 2 {
            score += version_numbers[2].parse::<i32>().unwrap();
        }

        std::cmp::Reverse(score)
    });

    let authlib_metadata_futures = authlib_versions.iter().map(|version| {
        let client = &http_client;
        let full_version = authlib_versions_to_full_versions.get(&version.to_string()).unwrap();
        async move {
            let url = authlib_download_url_format.replace("{}", full_version);
            let response = client.get(&url).send().await?.bytes().await?;
            let sha1 = hex::encode(Sha1::digest(&response));
            let size = response.len();
            Ok::<LibraryOverrideMetadata, Error>(LibraryOverrideMetadata {
                target_version: version.to_string(),
                name: format!("by.ely:authlib:{}", full_version),
                url,
                sha1,
                size
            })
        }
    });

    let authlib_metadatas = futures::future::join_all(authlib_metadata_futures).await;

    let mut json = json::JsonValue::new_object();
    let mut overrides = json::JsonValue::new_object();
    for metadata_result in authlib_metadatas {
        match metadata_result {
            Ok(metadata) => {
                overrides.insert(&metadata.target_version, json::object! {
                    name: metadata.name,
                    url: metadata.url,
                    sha1: metadata.sha1,
                    size: metadata.size
                }).unwrap();
            }
            Err(why) => {
                eprintln!("Couldn't create library metadata: {}", why);
                continue;
            }
        }
    }

    json["overrides"]["com.mojang:authlib"] = overrides;

    match injector_download.await {
        Ok(_) => {
            json["extras"]["authlib-injector"] = json::JsonValue::from(injector_download_url.to_string());
        }
        Err(why) => {
            eprintln!("Couldn't retrieve authlib-injector: {}", why);
            return;
        }
    }

    std::fs::write(output_file, json::stringify_pretty(json, 2)).unwrap();
}

struct LibraryOverrideMetadata {
    target_version: String,
    name: String,
    url: String,
    sha1: String,
    size: usize
}
