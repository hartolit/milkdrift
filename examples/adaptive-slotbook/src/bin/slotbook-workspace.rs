//! Small native workspace tool used to inspect candidate bytes and maintain editable instructions.
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let action = args.next().ok_or("expected knowledge or inspect")?;
    let root = PathBuf::from(args.next().ok_or("expected working directory")?);
    let note = args.next();
    if args.next().is_some() || (action != "note" && note.is_some()) {
        return Err("unexpected argument".into());
    }
    match action.as_str() {
        "knowledge" => {
            let mut file = match fs::File::create_new(root.join("KNOWLEDGE.md")) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    println!("retained existing KNOWLEDGE.md");
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            file.write_all(
                b"# Slotbook working knowledge\n\nPurpose: construct one booking product under the selected method and operator obligations.\n\nOperate through the granted workspace worker. Export exact candidate bytes, obtain the protected verifier result, and retain failures before attempting repairs. Public browsing does not permit anonymous mutation. Prototype language does not remove durable booking requirements.\n\nThe source failure and repair are seeded fixtures. Model planning decisions are unknown unless separately retained. No learned guidance is approved by this file. Select immutable evidence and an exact comparison/promotion receipt before claiming approval. Editable notes never change a prepared task's selected context.\n\nLimitations: finite synthetic HTTP checks, orderly restart only, no payment or external identity integration. Scratch tools require a recorded, staged recipe update before maintained use.\n",
            )?;
            println!(
                "created KNOWLEDGE.md; immutable selection remains a separate authorized operation"
            );
        }
        "note" => {
            let note = note.ok_or("note text required")?;
            let path = root.join("KNOWLEDGE.md");
            if note.is_empty()
                || note.len() > 4096
                || fs::metadata(&path)?.len() + note.len() as u64 + 2 > 65536
            {
                return Err("knowledge note exceeds bounded file size".into());
            }
            writeln!(fs::OpenOptions::new().append(true).open(path)?, "\n{note}")?;
        }
        "inspect" => {
            let mut file = fs::File::open(root.join("app"))?.take(4 * 1024 * 1024 + 1);
            let mut hash = blake3::Hasher::new();
            let bytes = std::io::copy(&mut file, &mut hash)?;
            if bytes == 0 || bytes > 4 * 1024 * 1024 {
                return Err("candidate outside finite native fixture bounds".into());
            }
            println!(
                "{}",
                serde_json::json!({"candidate_digest":format!("b3_{}",hash.finalize()),"bytes":bytes,"acceptance":"requires the independent protected verifier"})
            );
        }
        _ => return Err("expected knowledge or inspect".into()),
    }
    Ok(())
}
