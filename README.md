# Seiren AEC — Pure Rust Stereo Echo Canceller for Windows

Ứng dụng PC siêu nhẹ viết thuần bằng **Rust** (giao diện native `egui`, không cần Electron/WebView2 hay .NET runtime) giúp **triệt tiêu tiếng loa (game, nhạc, Discord) lọt vào micro** khi chơi game dùng loa ngoài và gọi thoại.

## Tại sao cần App này?
- Các bộ lọc tạp âm như Discord Krisp chỉ lọc tiếng ồn nền (quạt, gõ phím) chứ không lấy âm thanh hệ thống (game/nhạc) làm mẫu tham chiếu để trừ khử.
- **Seiren AEC** thu tín hiệu **WASAPI Loopback Stereo (2 kênh)** trực tiếp từ loa, đưa vào lõi **WebRTC Echo Canceller 3 (AEC3)** thuần Rust để mô hình hóa riêng âm học loa trái & loa phải, triệt tiêu tiếng loa dội vào micro mà vẫn giữ trọn chất giọng tự nhiên của micro USB (như Razer Seiren Mini).

## Tính năng chính
- **Giao diện Native siêu nhẹ (`eframe` / `egui`)**: Chế độ tối hiện đại, thanh đo mức tín hiệu thời gian thực (Loa / Mic gốc / Mic đã lọc), chỉ vẽ lại khi đang chạy (0% GPU/CPU khi dừng).
- **Stereo AEC3 (`aec3`)**: Hỗ trợ loa Stereo 2 kênh -> Mic Mono 1 kênh @ 48 kHz, tinh chỉnh chống mất chữ khi nói đè lên tiếng loa (*double-talk*).
- **Tùy chọn lọc nhiễu phụ**: Hỗ trợ bật/tắt **RNNoise** (`nnnoiseless` - AI lọc tiếng gõ phím cơ), **WebRTC NS**, **AGC2**, và tinh chỉnh độ trễ ban đầu.
- **Tích hợp nút Ghi thử 10s**: So sánh ngay file `seiren_test_raw.wav` (trước lọc) và `seiren_test.wav` (sau lọc) trên Desktop mà không cần cài VB-Cable trước.

## Cách sử dụng với Discord
1. Cài đặt **VB-Audio Virtual Cable** (miễn phí) tại https://vb-audio.com/Cable/ (chạy `VBCABLE_Setup_x64.exe` bằng quyền Administrator).
2. Mở `seiren-aec.exe` (double-click trực tiếp, không hiện cửa sổ CMD):
   - **Loa tham chiếu**: Chọn loa chính của bạn (ví dụ `Speakers (FiiO K11 R2R)`).
   - **Micro thu vào**: Chọn micro của bạn (ví dụ `Microphone (Razer Seiren Mini)`).
   - **Mic ảo đầu ra**: Chọn `CABLE Input (VB-Audio Virtual Cable)`.
   - Bấm **▶ BẮT ĐẦU LỌC**.
3. Mở **Discord -> Settings -> Voice & Video**:
   - **Input Device**: Chọn `CABLE Output (VB-Audio Virtual Cable)`.
   - **Output Device**: Giữ nguyên loa thật của bạn.

## Tự build từ mã nguồn
Yêu cầu Rust (`x86_64-pc-windows-gnu` + MinGW-w64 hoặc `x86_64-pc-windows-msvc` + VS C++ Build Tools):
```powershell
cargo build --release
```
File thực thi sẽ nằm tại `target\release\seiren-aec.exe`.

## Cấu trúc mã nguồn
- `src/main.rs`: Điểm vào chương trình (mặc định mở GUI, hỗ trợ thêm chế độ `--cli`).
- `src/gui.rs`: Giao diện native `eframe`/`egui`, chọn thiết bị, thanh đo VU thời gian thực.
- `src/engine.rs`: Quản lý 3 luồng âm thanh và vòng lặp DSP 10ms dùng lock-free `ringbuf`.
- `src/audio.rs`: Giao tiếp Windows WASAPI (liệt kê thiết bị, Stereo Loopback, Mic Capture, Virtual Mic Render).
- `src/dsp.rs`: Lõi xử lý Stereo WebRTC AEC3 + tùy chọn RNNoise.
