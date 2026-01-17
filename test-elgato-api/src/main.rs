
use elgato_streamdeck as streamdeck;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Check API version
    println!("elgato-streamdeck version: {}", streamdeck::VERSION);
    
    // List devices
    println!("Listing devices:");
    let devices = streamdeck::list_devices()?;
    println!("Devices found: {:?}", devices);
    
    if let Some((kind, serial)) = devices.first() {
        println!("First device: {:?}, Serial: {}", kind, serial);
        
        // Open device
        let mut device = streamdeck::StreamDeck::open(&kind, serial)?;
        
        // Check product info
        println!("Product: {:?}", device.product());
        println!("Firmware: {:?}", device.firmware_version());
        
        // Check display capabilities
        println!("Key image format: {:?}", device.kind().key_image_format());
        println!("LCD image format: {:?}", device.kind().lcd_image_format());
        println!("Key count: {} x {}", device.kind().column_count(), device.kind().row_count());
        
        // Check if it's a Stream Deck Plus
        if device.kind() == streamdeck::info::Kind::Plus {
            println!("This is a Stream Deck Plus!");
        }
    } else {
        println!("No Stream Deck devices found.");
    }
    
    Ok(())
}
