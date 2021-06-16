use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use extract_frontmatter::Extractor;
use handlebars::Handlebars;
use log::{error, info};
use maplit::{btreemap, hashmap};
use serde_derive::{Deserialize, Serialize};
use structopt::StructOpt;
use tempfile::NamedTempFile;
use tokio::process::Command;

fn default_address() -> String {
    "http://localhost:1181".to_string()
}

#[derive(Debug, Deserialize)]
struct PageMetadata {
    title: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_address")]
    server_address: String,

    username: Option<String>,
    password: Option<String>,

    token: Option<String>,
}

#[derive(Debug, StructOpt)]
enum Subcommand {
    AddUser { username: String, password: String },
    Auth,
    SetPage { slug: String },
}

#[derive(Debug, StructOpt)]
#[structopt(name = "uwiki-cli", about = "CLI to administer uwiki installations")]
struct Args {
    /// Configuration file. ~/.config/uwiki-cli/config.toml if not present.
    #[structopt(long, parse(from_os_str))]
    config_file: Option<PathBuf>,

    #[structopt(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, Deserialize)]
struct AddUserResponse {
    success: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct AuthenticateResponse {
    success: bool,
    message: String,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetPageResponse {
    success: bool,
    message: String,
    body: Option<String>,
    title: Option<String>,
    version: Option<i32>,
}

#[derive(Debug, Serialize)]
struct SetPageRequest {
    token: String,
    slug: String,
    title: String,
    body: String,
    previous_version: i32,
}

#[derive(Debug, Deserialize)]
struct SetPageResponse {
    success: bool,
    message: String,
    new_version: Option<i32>,
}

async fn cmd_add_user(username: String, password: String, config: Config) -> Result<()> {
    let map = hashmap! {
        "username" => username,
        "password" => password,
    };

    let response: AddUserResponse = reqwest::Client::new()
        .post(format!("{}/u", config.server_address))
        .json(&map)
        .send()
        .await
        .context("error sending request")?
        .json()
        .await
        .context("error parsing response JSON")?;

    if response.success {
        info!("{}", response.message);
    } else {
        error!("{}", response.message);
    }

    Ok(())
}

async fn cmd_auth(config: Config) -> Result<()> {
    let map = hashmap! {
        "username" => config
            .username
            .ok_or_else(|| anyhow!("config is missing username"))?,
        "password" => config
            .password
            .ok_or_else(|| anyhow!("config is missing password"))?,
    };

    let response: AuthenticateResponse = reqwest::Client::new()
        .post(format!("{}/a", config.server_address))
        .json(&map)
        .send()
        .await
        .context("error sending request")?
        .json()
        .await
        .context("error parsing response JSON")?;

    if response.success {
        match response.token {
            Some(token) => info!("Set 'token = \"{}\"' in your uwiki-cli config file", token),
            None => error!("Auth was successful, but no token was returned. Please retry."),
        }
    } else {
        error!("{}", response.message);
    }

    Ok(())
}

async fn cmd_set_page(slug: String, config: Config) -> Result<()> {
    let token = config
        .token
        .ok_or_else(|| anyhow!("No token set in config file"))?;
    let map = hashmap! {
        "token" => token.clone(),
        "slug" => slug.clone(),
    };

    let response: GetPageResponse = reqwest::Client::new()
        .post(format!("{}/g", config.server_address))
        .json(&map)
        .send()
        .await
        .context("error sending request")?
        .json()
        .await
        .context("error parsing response JSON")?;

    if !response.success {
        return Err(anyhow!("Error getting page from server"));
    }

    let previous_version = match response.version {
        Some(version) => version,
        None => {
            return Err(anyhow!("Server failed to return page version"));
        }
    };

    let mut file = NamedTempFile::new()?;
    let source = "---\ntitle: {{#if title}}{{title}}{{/if}}\n---\n{{#if body}}{{body}}{{/if}}";
    let mut handlebars = Handlebars::new();

    handlebars.register_template_string("t1", source)?;

    let data = btreemap! {
        "title" => response.title,
        "body" => response.body,
    };
    let text = handlebars.render("t1", &data)?;

    file.write_all(text.as_bytes())?;

    let exit_status = Command::new(std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string()))
        .arg(file.path())
        .spawn()?
        .wait()
        .await?;

    // TODO(jsvana): make all errors after editor dump the file
    // and log to user
    if !exit_status.success() {
        let (_, path) = file.keep()?;
        info!(
            "Editor exited with nonzero code. Refusing to continue. \
            Edited content is accessible at \"{}\".",
            path.display()
        );

        return Ok(());
    }

    let mut contents = String::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_string(&mut contents)?;

    let mut extractor = Extractor::new(&contents);
    extractor.select_by_terminator("---");

    let (front_matter, body) = extractor.split();
    let front_matter = front_matter.join("\n");
    let metadata: PageMetadata = serde_yaml::from_str(&front_matter)?;

    let request = SetPageRequest {
        token,
        slug,
        title: metadata.title,
        body: body.to_string(),
        previous_version,
    };

    let response: SetPageResponse = reqwest::Client::new()
        .post(format!("{}/s", config.server_address))
        .json(&request)
        .send()
        .await
        .context("error sending request")?
        .json()
        .await
        .context("error parsing response JSON")?;

    if response.success {
        info!("{}", response.message);
    } else {
        error!("{}", response.message);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    pretty_env_logger::init();

    let args = Args::from_args();

    let config_file = match args.config_file {
        Some(config_file) => config_file,
        None => {
            let dirs = xdg::BaseDirectories::with_prefix("uwiki-cli")?;
            dirs.find_config_file("config.toml")
                .ok_or_else(|| anyhow!("no uwiki-cli config file found in .config/uwiki-cli"))?
        }
    };

    let config: Config = toml::from_str(
        &std::fs::read_to_string(config_file.clone())
            .with_context(|| anyhow!("failed to read config file at {:?}", config_file))?,
    )
    .with_context(|| anyhow!("failed to parse config file at {:?}", config_file))?;

    match args.subcommand {
        Subcommand::AddUser { username, password } => {
            cmd_add_user(username, password, config).await
        }
        Subcommand::Auth => cmd_auth(config).await,
        Subcommand::SetPage { slug } => cmd_set_page(slug, config).await,
    }
}
