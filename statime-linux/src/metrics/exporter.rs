use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct Args {
    /// Configuration file to use
    #[clap(
        long = "config",
        short = 'c',
        default_value = "/etc/statime/statime.toml"
    )]
    config: Option<PathBuf>,
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Args::parse();
    dbg!(options);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use crate::metrics::exporter::Args;

    const BINARY: &str = "/usr/bin/statime-metrics-exporter";

    #[test]
    fn cli_config() {
        let config_str = "/foo/bar/statime.toml";
        let config = Path::new(config_str);
        let arguments = &[BINARY, "-c", config_str];

        let options = Args::try_parse_from(arguments).unwrap();
        assert_eq!(options.config.unwrap().as_path(), config);
    }
}
