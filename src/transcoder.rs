use bytes::Bytes;
use anyhow::Result;

pub struct FrameTranscoder {
}

impl FrameTranscoder {
    pub fn new() -> Self {
        Self {}
    }


    pub async fn create_test_frame(&self) -> Result<Bytes> {
        Ok(Bytes::from(self.create_test_jpeg()))
    }

    pub async fn create_test_frame_rtsp_connected(&self) -> Result<Bytes> {
        Ok(Bytes::from(self.create_rtsp_connected_jpeg()))
    }

    fn create_test_jpeg(&self) -> Vec<u8> {
        use image::{ImageBuffer, Rgb};
        
        let width = 640u32;
        let height = 480u32;
        
        let img = ImageBuffer::from_fn(width, height, |x, y| {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u32;
            
            let r = ((x + t / 10) % 255) as u8;
            let g = ((y + t / 20) % 255) as u8;
            let b = (((x + y + t / 5) % 255)) as u8;
            
            Rgb([r, g, b])
        });
        
        let mut jpeg_data = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut jpeg_data);
            img.write_to(&mut cursor, image::ImageFormat::Jpeg)
                .expect("Failed to encode JPEG");
        }
        
        jpeg_data
    }

    fn create_rtsp_connected_jpeg(&self) -> Vec<u8> {
        use image::{ImageBuffer, Rgb};
        
        let width = 640u32;
        let height = 480u32;
        
        let img = ImageBuffer::from_fn(width, height, |x, y| {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u32;
            
            // Different color pattern to indicate RTSP connection
            let r = ((x + t / 5) % 255) as u8;  // Faster red
            let g = 128 + ((y + t / 10) % 127) as u8;  // More green
            let b = ((t / 15) % 255) as u8;  // Blue based on time only
            
            Rgb([r, g, b])
        });
        
        let mut jpeg_data = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut jpeg_data);
            img.write_to(&mut cursor, image::ImageFormat::Jpeg)
                .expect("Failed to encode JPEG");
        }
        
        jpeg_data
    }
}