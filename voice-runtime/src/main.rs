use ptts::{
    checkpoint::{Checkpoint, ResolveOptions},
    preprocess::Lang,
    synth::{DeviceKind, Quant},
};
use std::{
    error::Error,
    io::{self, BufRead, Read, Write},
    path::PathBuf,
};

fn main() -> Result<(), Box<dyn Error>> {
    let model = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("/usr/share/enzimos/voice/phonon-31m-en"),
        PathBuf::from,
    );
    let checkpoint = Checkpoint::resolve(
        &model,
        ResolveOptions {
            quant: Quant::Q80,
            weights: None,
        },
    )?;
    let synth = checkpoint
        .builder(Lang::En)
        .device(DeviceKind::Cpu)
        .condition("duration_delta", "-0.5")
        .add_voice("default", model.join("default-voice.safetensors"))
        .voice("default")
        .build()?;
    if synth.sample_rate() != 24_000 {
        return Err("unexpected Phonon sample rate".into());
    }
    let input = io::stdin().lock();
    let mut input = io::BufReader::new(input);
    let mut output = io::BufWriter::new(io::stdout().lock());
    output.write_all(b"EV01")?;
    output.flush()?;
    loop {
        let mut line = String::new();
        if input.by_ref().take(1025).read_line(&mut line)? == 0 {
            return Ok(());
        }
        let Some(text) = line.strip_suffix('\n') else {
            return Err("invalid synthesis request".into());
        };
        for chunk in synth.stream(text)? {
            let pcm: Vec<_> = chunk?
                .iter()
                .map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16)
                .flat_map(i16::to_le_bytes)
                .collect();
            output.write_all(&u32::try_from(pcm.len())?.to_le_bytes())?;
            output.write_all(&pcm)?;
            output.flush()?;
        }
        output.write_all(&0u32.to_le_bytes())?;
        output.flush()?;
    }
}
