use base64::decode;
use csv::Writer;
use progress_bar::*;
use serde::{Deserialize, Serialize};
use serde_aux::prelude::*;
use serde_json::to_string_pretty;
use std::collections::HashMap;
use std::error::Error;
use std::str;
use std::str::FromStr;
use std::{env, fs};
use web3::contract::{Contract, Options};
// use web3::futures::Future;
use web3::transports::{batch, http};
use web3::types::{Address, U256};
use web3::BatchTransport;

#[derive(Debug, Deserialize, Serialize)]
struct TokenError {
    token_id: i32,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Attribute {
    trait_type: String,
    #[serde(deserialize_with = "deserialize_string_from_number")]
    value: String,
    display_type: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
struct Metadata {
    name: String,
    attributes: Vec<Attribute>,
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let file =
        fs::read_to_string("./in/addresses.txt").expect("Error reading 'addresses.txt' file.");
    let addresses = file.split("\n");
    let base_path = env::current_dir()?;
    for a in addresses {
        let out_path = format!("{}/{}", base_path.to_str().unwrap(), a);
        match get_metadata(a).await {
            Ok(data) => {
                println!(
                    "Successfully Data Mined Contract {}. Creating Output Files.",
                    a
                );
                fs::create_dir_all(&out_path)?;
                match std::fs::write(
                    format!("{}/metadata.json", &out_path),
                    to_string_pretty(&data)?,
                ) {
                    Ok(()) => println!("JSON File made for {}", a),
                    Err(err) => println!(
                        "ERROR: Could not make JSON file for {}\nMessage: {}",
                        a, err
                    ),
                };
                match write_csv(data, format!("{}/metadata.csv", &out_path)) {
                    Ok(()) => println!("CSV File made for {}", a),
                    Err(err) => {
                        println!("ERROR: Could not make CSV file for {}\nMessage: {}", a, err)
                    }
                }
            }
            Err(()) => println!("Failed to Data Mine Contract {}. Please Check Address.", a),
        }
    }
    Ok(())
}

fn write_csv(data: HashMap<i32, Metadata>, path: String) -> Result<(), Box<dyn Error>> {
    let mut headers: Vec<String> = vec![String::from("tokenId"), String::from("name")];
    let mut t_map: HashMap<&i32, HashMap<String, String>> = HashMap::new();

    data.keys().for_each(|id| {
        let meta: &Metadata = data.get(id).unwrap();
        let mut t_m: HashMap<String, String> = HashMap::new();
        t_m.insert("tokenId".to_string(), id.to_string());
        t_m.insert("name".to_string(), data.get(id).unwrap().name.to_string());

        for attr in meta.attributes.iter() {
            let a: &Attribute = attr.clone();
            t_m.insert(a.trait_type.to_string(), a.value.to_string());
            if !headers.contains(&a.trait_type) {
                headers.push(a.trait_type.clone());
            }
        }
        t_map.insert(id, t_m);
    });
    let mut wtr = Writer::from_path(path)?;
    wtr.write_record(&headers)?;
    for id in data.keys() {
        let temp_map: &HashMap<String, String> = t_map.get(id).unwrap();
        let mut rows: Vec<String> = Vec::new();
        for h in headers.iter() {
            let row = match temp_map.get(h) {
                Some(row) => row.to_string(),
                None => "".to_string(),
            };
            rows.push(row);
        }
        wtr.write_record(&rows)?;
    }

    wtr.flush()?;
    Ok(())
}

async fn get_metadata(address: &str) -> Result<HashMap<i32, Metadata>, ()> {
    let con_addr: Address = Address::from_str(address).unwrap();
    let contract: Contract<http::Http> = get_contract(con_addr).unwrap();
    println!("Got contract");
    let mut id_map: HashMap<i32, Metadata> = HashMap::new();
    let mut headers: Vec<String> = vec![String::from("tokenId"), String::from("name")];
    let total_supply: i32 = u256_to_i32(
        contract
            .query("totalSupply", (), None, Options::default(), None)
            .await
            .unwrap(),
    );
    println!("Supply: {}", total_supply);
    let mut count: i32 = 0;
    let mut left: i32 = total_supply;
    init_progress_bar(total_supply as usize);
    set_progress_bar_action(address, Color::Blue, Style::Bold);
    loop {
        if left == 0 {
            finalize_progress_bar();
            println!("Finished");
            break;
        }
        let token_uri: Result<String, web3::contract::Error> = contract
            .query(
                "tokenURI",
                U256::from(count),
                None,
                Options::default(),
                None,
            )
            .await;
        match token_uri {
            Ok(uri) => {
                match req_token_uri(uri).await {
                    Ok(data) => {
                        for t in data.attributes.iter().map(|x| &x.trait_type) {
                            let t_type: String = t.clone();
                            if !headers.contains(&t_type) {
                                headers.push(t_type);
                            }
                        }
                        id_map.insert(count, data);
                    }
                    Err(err) => print_progress_bar_info(
                        "Failure",
                        format!("ERROR: {}\ntokenId: {}", err, count).as_str(),
                        Color::Red,
                        Style::Bold,
                    ),
                }
                left -= 1;
                inc_progress_bar();
            }
            Err(_) => (),
        }
        count += 1;
    }
    println!("{:#?}", headers);
    Ok(id_map)
}
async fn req_token_uri(uri: String) -> Result<Metadata, Box<dyn Error>> {
    if uri.starts_with("data") && uri.contains("base64") {
        let split: Vec<&str> = uri.split("base64,").collect();
        let bytes = decode(split[split.len() - 1])?;
        let decoded: Metadata = serde_json::from_str(String::from_utf8(bytes)?.as_str())?;
        return Ok(decoded);
    }
    let req_uri: String = if uri.contains("ipfs://") {
        let split: Vec<&str> = uri.split("ipfs://").collect();
        format!(
            "https://gateway.pinata.cloud/ipfs/{}",
            split[split.len() - 1]
        )
    } else {
        format_uri(uri)
    };
    let res: Metadata = reqwest::Client::new()
        .get(req_uri)
        .send()
        .await?
        .json()
        .await?;
    Ok(res)
}
fn get_contract(a: Address) -> Result<Contract<http::Http>, web3::Error> {
    let http = web3::transports::Http::new("https://api.mycryptoapi.com/eth").unwrap();
    let web3h = web3::Web3::new(http);

    let contract: Contract<http::Http> =
        Contract::from_json(web3h.eth(), a, include_bytes!("baseABI.json")).unwrap();

    Ok(contract)
}
// https://api.mycryptoapi.com/eth
fn u256_to_i32(wei_val: U256) -> i32 {
    wei_val.as_u128() as i32
}
fn format_uri(uri: String) -> String {
    if uri.starts_with("https://") {
        uri
    } else {
        format!("https://{}", uri)
    }
}
