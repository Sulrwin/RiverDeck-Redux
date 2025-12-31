use app_core::ids::DeviceId;
use device::DeviceService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "list" => cmd_list().await,
        "events" => cmd_events(&args).await,
        "brightness" => cmd_brightness(&args).await,
        "test-image" => cmd_test_image(&args).await,
        other => anyhow::bail!("unknown command: {other} (run `cli help`)"),
    }
}

fn print_help() {
    eprintln!(
        r#"riverdeck-redux cli

USAGE:
  cli list
  cli events <device_id>
  cli brightness <device_id> <percent>
  cli test-image <device_id> <key> <r> <g> <b>
"#
    );
}

async fn cmd_list() -> anyhow::Result<()> {
    let svc = device::HidDeviceService::new()?;
    let devices = svc.list_devices().await?;
    for d in devices {
        println!("{}  {}", d.id.0, d.display_name);
    }
    Ok(())
}

async fn cmd_events(args: &[String]) -> anyhow::Result<()> {
    let id = parse_device_id(args, 2)?;
    let svc = device::HidDeviceService::new()?;
    let mut dev = svc.connect(id).await?;

    println!("connected: {} (keys: {})", dev.name, dev.key_count);
    while let Some(ev) = dev.events.recv().await {
        println!("{ev:?}");
    }
    Ok(())
}

async fn cmd_brightness(args: &[String]) -> anyhow::Result<()> {
    let id = parse_device_id(args, 2)?;
    let percent: u8 = parse_u8(args, 3)?;

    let svc = device::HidDeviceService::new()?;
    let dev = svc.connect(id).await?;
    dev.set_brightness(percent).await?;
    println!("brightness set to {percent}%");
    Ok(())
}

async fn cmd_test_image(args: &[String]) -> anyhow::Result<()> {
    let id = parse_device_id(args, 2)?;
    let key: u8 = parse_u8(args, 3)?;
    let r: u8 = parse_u8(args, 4)?;
    let g: u8 = parse_u8(args, 5)?;
    let b: u8 = parse_u8(args, 6)?;

    let svc = device::HidDeviceService::new()?;
    let dev = svc.connect(id).await?;

    let (w, h) = match dev.key_count {
        6 => (80, 80),  // common mini size
        32 => (96, 96), // common XL size
        _ => (72, 72),  // common original/mk2 size
    };

    let jpeg = render::test_patterns::solid_color_jpeg(w, h, [r, g, b])?;
    dev.set_key_image_jpeg(key, jpeg).await?;
    println!("pushed test image to key {key}");
    Ok(())
}

fn parse_device_id(args: &[String], idx: usize) -> anyhow::Result<DeviceId> {
    let raw = args
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("missing device_id"))?;
    Ok(DeviceId(raw.parse::<u64>()?))
}

fn parse_u8(args: &[String], idx: usize) -> anyhow::Result<u8> {
    let raw = args
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("missing arg {idx}"))?;
    Ok(raw.parse::<u8>()?)
}
