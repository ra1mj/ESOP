use esop_cfggen::{GeneratorError, generate};
use std::env;
use std::path::PathBuf;

fn parse_args() -> Result<(PathBuf, PathBuf), GeneratorError> {
    let mut input = None;
    let mut output = None;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--input" => {
                input = arguments.next().map(PathBuf::from);
            }
            "--output" => {
                output = arguments.next().map(PathBuf::from);
            }
            "-h" | "--help" => {
                return Err(GeneratorError::Usage(
                    "usage: esop-cfggen --input <product.json> --output <directory>".to_owned(),
                ));
            }
            _ => {
                return Err(GeneratorError::Usage(format!(
                    "unknown argument {argument:?}; usage: esop-cfggen --input <product.json> --output <directory>"
                )));
            }
        }
    }
    let input = input.ok_or_else(|| GeneratorError::Usage("missing --input".to_owned()))?;
    let output = output.ok_or_else(|| GeneratorError::Usage("missing --output".to_owned()))?;
    Ok((input, output))
}

fn main() {
    let result = parse_args().and_then(|(input, output)| {
        let summary = generate(&input, &output)?;
        println!(
            "generated {} product artifacts at {} (config {})",
            summary.artifact_count,
            output.display(),
            summary.config_sha256
        );
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("esop-cfggen: {error}");
        std::process::exit(2);
    }
}
