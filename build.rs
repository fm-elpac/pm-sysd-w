use std::error::Error;

#[cfg(feature = "cli")]
use pm_bin::build_gen;

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(feature = "cli")]
    {
        build_gen(Some(".".into()))?;
    }
    Ok(())
}
