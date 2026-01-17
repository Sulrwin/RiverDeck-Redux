
use elgato_streamdeck::StreamDeck;
use image::RgbImage;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut stream_decks = elgato_streamdeck::list_devices()?;
    if stream_decks.is_empty() {
        anyhow::bail!("No Stream Decks found!");
    }
    println!("Found {} devices!", stream_decks.len());

    for (i, device_info) in stream_decks.iter().enumerate() {
        println!("Device {}: {:?}", i, device_info.product);
        println!("  Serial Number: {:?}", device_info.serial_number);
        println!("  Vendor ID: 0x{:04X}, Product ID: 0x{:04X}", device_info.vendor_id, device_info.product_id);
        println!("  Input report length: {}", device_info.input_report_length);
        println!("  Output report length: {}", device_info.output_report_length);
    }

    // Open the first device
    let mut stream_deck = StreamDeck::open(&stream_decks[0])?;
    println!("Opened device: {:?}", stream_decks[0].product);

    // Create a red image for testing
    let key_count = stream_deck.key_count();
    let key_size = stream_deck.key_image_size();
    println!("Key count: {}, Key size: {:?}", key_count, key_size);
    
    let mut img = RgbImage::new(key_size.0, key_size.1);
    for x in 0..key_size.0 {
        for y in 0..key_size.1 {
            img.put_pixel(x, y, image::Rgb([255, 0, 0]));
        }
    }
    
    // Set the image on key 0
    stream_deck.set_key_image(0, img)?;
    println!("Set red image to key 0!");

    // Wait for 2 seconds
    std::thread::sleep(Duration::from_secs(2));

    // Clear the key
    let black_img = RgbImage::new(key_size.0, key_size.1);
    stream_deck.set_key_image(0, black_img)?;
    println!("Cleared key 0!");

    Ok(())
}
