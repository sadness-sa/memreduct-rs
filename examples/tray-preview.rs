//! Render the actual Windows numeric icons at native tray DPI sizes for visual QA.
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use memreduct_rs::{platform::wide, tray::number_icon};
    use std::{
        io::Write,
        mem::{size_of, zeroed},
        ptr::null_mut,
    };
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
    unsafe {
        let width: i32 = 360;
        let height: i32 = 174;
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        assert!(!dc.is_null() && !bitmap.is_null() && !bits.is_null());
        let old_bitmap = SelectObject(dc, bitmap);
        let brush = CreateSolidBrush(0x00fafafa);
        FillRect(
            dc,
            &RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            brush,
        );
        DeleteObject(brush);
        let font = CreateFontW(
            -13,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let old_font = SelectObject(dc, font);
        SetTextColor(dc, 0x00444444);
        SetBkMode(dc, TRANSPARENT as i32);
        for (row, size) in [16, 24, 32].iter().copied().enumerate() {
            let y = 16 + row as i32 * 52;
            let mut label = RECT {
                left: 12,
                top: y,
                right: 62,
                bottom: y + 32,
            };
            DrawTextW(
                dc,
                wide(&format!("{size}px")).as_ptr(),
                -1,
                &mut label,
                DT_VCENTER | DT_SINGLELINE,
            );
            for (col, value) in [0, 9, 70, 85, 100].iter().copied().enumerate() {
                let icon = number_icon(Some(value), size, 85)?;
                DrawIconEx(
                    dc,
                    80 + col as i32 * 52,
                    y + (32 - size) / 2,
                    icon.0,
                    size,
                    size,
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
        }
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize);
        for pixel in pixels.iter_mut() {
            *pixel |= 0xff000000;
        }
        std::fs::create_dir_all("dist")?;
        let mut out = std::fs::File::create("dist/tray-preview.bmp")?;
        let bytes = (width * height * 4) as u32;
        out.write_all(b"BM")?;
        out.write_all(&(bytes + 54).to_le_bytes())?;
        out.write_all(&[0u8; 4])?;
        out.write_all(&54u32.to_le_bytes())?;
        out.write_all(&40u32.to_le_bytes())?;
        out.write_all(&width.to_le_bytes())?;
        out.write_all(&(-height).to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?;
        out.write_all(&32u16.to_le_bytes())?;
        out.write_all(&[0u8; 4])?;
        out.write_all(&bytes.to_le_bytes())?;
        out.write_all(&[0u8; 16])?;
        out.write_all(std::slice::from_raw_parts(
            bits.cast::<u8>(),
            bytes as usize,
        ))?;
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        DeleteObject(font);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    Ok(())
}
