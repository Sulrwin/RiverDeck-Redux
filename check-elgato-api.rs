
// Simple program to check elgato-streamdeck API
fn main() {
    println!("Checking elgato-streamdeck crate API...");
    println!("elgato-streamdeck version: {}", elgato_streamdeck::VERSION);
    
    // List all public items in elgato_streamdeck module
    println!("\nTop-level items: {:?}", std::module_path!());
    
    // Check if AsyncStreamDeck exists
    println!("\nAsyncStreamDeck: {:?}", std::any::type_name::<elgato_streamdeck::AsyncStreamDeck>());
    
    // Check if HidApi exists
    println!("\nHidApi: {:?}", std::any::type_name::<elgato_streamdeck::HidApi>());
    
    // Check if list_devices_async exists
    println!("\nlist_devices_async: {:?}", std::any::type_name::<elgato_streamdeck::list_devices_async>());
    
    // Check images module
    println!("\nImages module: {:?}", std::any::type_name::<elgato_streamdeck::images>());
}
