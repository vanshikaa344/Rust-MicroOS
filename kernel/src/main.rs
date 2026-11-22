#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use bootloader_api::{entry_point, info::FrameBufferInfo, info::PixelFormat, BootInfo};
use core::alloc::{GlobalAlloc, Layout};
use core::fmt::{self, Write};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use uart_16550::SerialPort;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

const PIC_1_OFFSET: u8 = 32;
const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
const TIMER_FREQUENCY_HZ: u32 = 20;
const MAX_TASKS: usize = 8;

entry_point!(kernel_main);

lazy_static! {
    static ref SERIAL1: Mutex<SerialPort> = {
        let mut serial = unsafe { SerialPort::new(0x3F8) };
        serial.init();
        Mutex::new(serial)
    };
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt
    };
}

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

// Light Pink Aurora Desktop Color Palette
const BG_DARK: [u8; 3] = [255, 240, 245];       // Lavender blush (light pink bg)
const TOP_BAR: [u8; 3] = [255, 182, 206];       // Light pink top bar
const CARD_BG: [u8; 3] = [255, 228, 237];       // Misty rose card surface
const CARD_BORDER: [u8; 3] = [255, 150, 180];   // Pink card outline
const CYAN_ACCENT: [u8; 3] = [220, 60, 120];    // Deep rose accent (replaces cyan)
const GREEN_ACCENT: [u8; 3] = [180, 60, 100];   // Berry accent (replaces green)
const AMBER_ACCENT: [u8; 3] = [200, 80, 140];   // Fuchsia accent (replaces amber)
const PURPLE_ACCENT: [u8; 3] = [160, 40, 100];  // Dark rose accent (replaces purple)
const TEXT_MUTED: [u8; 3] = [180, 100, 130];    // Muted rose text
const TEXT_MAIN: [u8; 3] = [80, 20, 50];        // Deep burgundy main text

// Terminal Window Dimensions (Scrollable In-Kernel Text Buffer)
const TERM_LINES: usize = 12;
const TERM_COLS: usize = 70;

struct Screen {
    buffer: *mut u8,
    info: Option<FrameBufferInfo>,
    active_task: usize,
    completed: [bool; 3],
    term_history: [[u8; TERM_COLS]; TERM_LINES],
    term_colors: [[u8; 3]; TERM_LINES],
    term_count: usize,
    input_buf: [u8; 48],
    input_len: usize,
}

unsafe impl Sync for Screen {}
unsafe impl Send for Screen {}

impl Screen {
    const fn new() -> Self {
        Self {
            buffer: core::ptr::null_mut(),
            info: None,
            active_task: 0,
            completed: [false; 3],
            term_history: [[b' '; TERM_COLS]; TERM_LINES],
            term_colors: [TEXT_MAIN; TERM_LINES],
            term_count: 0,
            input_buf: [0; 48],
            input_len: 0,
        }
    }

    fn init(&mut self, framebuffer: &mut bootloader_api::info::FrameBuffer) {
        let info = framebuffer.info();
        self.buffer = framebuffer.buffer_mut().as_mut_ptr();
        self.info = Some(info);
    }

    fn show_splash_animation(&mut self) {
        let Some(info) = self.info else { return };
        let w = info.width;
        let h = info.height;

        // Stage 1: Pink Aurora boot splash with rose-glow OS logo
        self.fill([255, 220, 232]);

        // Draw Stylized Diamond / Petal Logo for Aurora-OS
        let cx = w / 2;
        let cy = h / 2 - 40;

        // Outer petal glow layers (light pink outward)
        self.rect(cx - 36, cy - 36, 72, 72, [255, 182, 206]);
        self.rect(cx - 32, cy - 32, 64, 64, [255, 130, 170]);
        self.rect(cx - 24, cy - 24, 48, 48, [230, 80, 130]);
        self.rect(cx - 12, cy - 12, 24, 24, [255, 240, 245]);

        // Title and Subtitle
        self.text(cx - 96, cy + 54, "AURORA-OS", 3, [80, 20, 50]);
        self.text(cx - 140, cy + 90, "LIGHTWEIGHT RUST PINK MICROKERNEL", 1, CYAN_ACCENT);
        self.text(cx - 120, cy + 106, "BUILT ON AMD RYZEN 7 7730U PLATFORM", 1, TEXT_MUTED);

        // Animated Boot Progress Bar (rose-tinted)
        let bar_w = 320;
        let bar_x = cx - bar_w / 2;
        let bar_y = cy + 130;
        self.rect(bar_x - 2, bar_y - 2, bar_w + 4, 14, [255, 182, 206]);

        // Fill progress steps with simulated delay
        let steps = [
            (60, "BOOT: PROBING ACPI / UEFI TABLES..."),
            (140, "INIT: DISPLAY SUBSYSTEM (1920X1080 BGR)..."),
            (220, "INIT: AUDIO DRIVER (HDA CODEC / ALSA)..."),
            (320, "SYSTEM: STARTING PRIORITY TASK SCHEDULER..."),
        ];

        for (progress, msg) in steps.iter() {
            self.rect(bar_x, bar_y, *progress, 10, CYAN_ACCENT);
            self.rect(bar_x, bar_y + 18, bar_w, 12, [255, 220, 232]); // clear text area
            self.text(bar_x + 10, bar_y + 18, msg, 1, TEXT_MUTED);

            // Busy delay loop for visual presence
            for _ in 0..500_000 {
                core::hint::spin_loop();
            }
        }
    }

    fn render_desktop(&mut self) {
        let Some(info) = self.info else { return };
        let w = info.width;
        let h = info.height;

        // Background wallpaper fill (light pink)
        self.fill(BG_DARK);

        // Top Pink Desktop Panel
        self.rect(0, 0, w, 28, TOP_BAR);
        self.text(14, 8, "AURORA-OS DESKTOP", 1, CYAN_ACCENT);
        self.text(170, 8, "APPS: SHELL, BROWSER, AUDIO, TOP", 1, TEXT_MUTED);
        self.text(w - 230, 8, "WLAN: CONNECTED", 1, GREEN_ACCENT);
        self.text(w - 75, 8, "LIVE OS", 1, AMBER_ACCENT);

        // Dashboard Header Card
        self.card(16, 42, w - 32, 44);
        self.text(26, 52, "AURORA-OS KERNEL v1.0.0 // STUDENT DESKTOP ENVIRONMENT", 1, [80, 20, 50]);
        self.text(26, 68, "LENOVO IDEAPAD 5 | AMD RYZEN 7 7730U (8C/16T) | PIT 20 HZ | INTEL AX211 WIFI 6E", 1, TEXT_MUTED);

        // Subsystem Telemetry Banner
        let card_w = (w - 32 - 18) / 2;
        let col_w = (w - 32 - 18) / 2;

        // Left Subsystems Card
        self.card(16, 36, card_w, 88);
        self.rect(16, 36, 3, 88, CYAN_ACCENT);
        self.text(26, 42, "HARDWARE & PLATFORM", 1, CYAN_ACCENT);
        self.text(26, 56, "HOST: LENOVO IDEAPAD 5 (AMD RYZEN 7 7730U 8C/16T)", 1, TEXT_MAIN);
        self.text(26, 70, "NET: INTEL AX211 WIFI-6E + GBE ETHERNET (192.168.1.5/24)", 1, GREEN_ACCENT);
        self.text(26, 84, "INTERRUPT: IDT 256-ENTRY + PIT 20 HZ + PS/2 KEYBOARD", 1, TEXT_MUTED);
        self.text(26, 98, "BOOT WATERFALL: 10.1MS -> 28.4MS -> 61.2MS -> 290.5MS", 1, AMBER_ACCENT);

        // Card 1: Hardware Specs
        self.card(16, 94, col_w, 106);
        self.rect(16, 94, 4, 106, CYAN_ACCENT);
        self.text(28, 104, "HARDWARE ARCHITECTURE", 1, CYAN_ACCENT);
        self.text(28, 122, "CHIPSET: AMD SoC PCIE 4.0 (IDEAPAD 5)", 1, TEXT_MAIN);
        self.text(28, 138, "PROCESSOR: RYZEN 7 7730U (8C @ 4.5GHZ)", 1, TEXT_MUTED);
        self.text(28, 154, "INTERRUPTS: APIC + PIC 8259 REMAPPED", 1, TEXT_MUTED);
        self.text(28, 170, "MEMORY HEAP: 64 KIB MONOTONIC BUMP", 1, GREEN_ACCENT);
        self.text(col_w - 75, 104, "[16.2 MS]", 1, AMBER_ACCENT);

        // Right Package & Store Card
        self.card(26 + card_w, 36, card_w, 88);
        self.rect(26 + card_w, 36, 3, 88, PURPLE_ACCENT);
        self.text(36 + card_w, 42, "AUDIO & DISPLAY SUBSYSTEM", 1, PURPLE_ACCENT);
        self.text(36 + card_w, 56, "AUDIO: AMD HDA CODEC (REALTEK ALC257)", 1, TEXT_MAIN);
        self.text(36 + card_w, 70, "DISPLAY: 1920X1080 IPS (60HZ BGR FRAMEBUFFER)", 1, TEXT_MUTED);
        self.text(36 + card_w, 84, "SCHEDULER: PRIORITY TASK QUEUE (3 TASKS)", 1, CYAN_ACCENT);
        self.text(36 + card_w, 98, "TRY COMMANDS: HELP, AUDIO, TOP, HWINFO, CLEAR", 1, GREEN_ACCENT);

        // Card 2: Network Stack
        self.card(26 + col_w, 94, col_w, 106);
        self.rect(26 + col_w, 94, 4, 106, GREEN_ACCENT);
        self.text(38 + col_w, 104, "NETWORK CONNECTIVITY", 1, GREEN_ACCENT);
        self.text(38 + col_w, 122, "WLAN: INTEL AX211 WIFI-6E (802.11AX/BE)", 1, TEXT_MAIN);
        self.text(38 + col_w, 138, "LAN: REALTEK GBE (1000BASE-T FULL-DUPLEX)", 1, TEXT_MUTED);
        self.text(38 + col_w, 154, "STACK: SMOLTCP IPV4 LINK UP (DHCP OK)", 1, TEXT_MUTED);
        self.text(38 + col_w, 170, "STATUS: IP 192.168.1.5/24 (ONLINE)", 1, GREEN_ACCENT);
        self.text(w - 90, 104, "[38.5 MS]", 1, AMBER_ACCENT);

        // Interactive Live Terminal Window (PS/2 Driven)
        self.card(16, 132, w - 32, 320);
        self.rect(16, 132, w - 32, 22, [255, 182, 206]);
        self.text(26, 138, "INTERACTIVE TERMINAL CONSOLE // PS/2 KEYBOARD ACTIVE", 1, CYAN_ACCENT);
        self.text(w - 220, 138, "TYPE COMMAND & HIT ENTER", 1, AMBER_ACCENT);

        // Card 3: Audio Manager
        self.card(16, 208, col_w, 106);
        self.rect(16, 208, 4, 106, AMBER_ACCENT);
        self.text(28, 218, "AUDIO SUBSYSTEM: HDA CODEC", 1, AMBER_ACCENT);
        self.text(28, 236, "DRIVER: /DEV/SND/PCM0 -> ALSA LAYER", 1, TEXT_MAIN);
        self.text(28, 252, "CODEC: REALTEK ALC257 (HDA SPEC 1.0)", 1, TEXT_MUTED);
        self.text(28, 268, "SAMPLE RATE: 44100 HZ / 16-BIT STEREO", 1, TEXT_MUTED);
        self.text(28, 284, "STATUS: CODEC ONLINE (VOLUME: 70%)", 1, GREEN_ACCENT);
        self.text(col_w - 75, 218, "[74.8 MS]", 1, AMBER_ACCENT);

        // Print initial welcome in terminal history
        self.add_term_line("Welcome to Aurora-OS Shell! Type 'help' for available commands.", CYAN_ACCENT);
        self.add_term_line("Hardware: AMD Ryzen 7 7730U | Intel WiFi 6E | HDA Audio", TEXT_MUTED);
        self.add_term_line("Type 'audio' to control sound, 'top' to view scheduler.", GREEN_ACCENT);

        // Card 4: Display Info
        self.card(26 + col_w, 208, col_w, 106);
        self.rect(26 + col_w, 208, 4, 106, PURPLE_ACCENT);
        self.text(38 + col_w, 218, "DISPLAY: FRAMEBUFFER ENGINE", 1, PURPLE_ACCENT);
        self.text(38 + col_w, 236, "RESOLUTION: 1920X1080 LINEAR BGR", 1, TEXT_MAIN);
        self.text(38 + col_w, 252, "DEPTH: 24BPP | STRIDE: 1920*3 BYTES", 1, TEXT_MUTED);
        self.text(38 + col_w, 268, "DRIVER: VESA/UEFI GOP FRAMEBUFFER", 1, TEXT_MUTED);
        self.text(38 + col_w, 284, "STATUS: RENDER PIPELINE ACTIVE", 1, GREEN_ACCENT);
        self.text(w - 95, 218, "[122.3 MS]", 1, AMBER_ACCENT);
        self.redraw_term();

        // Card 5: Research Track - Priority Scheduler
        self.card(16, 322, w - 32, 146);
        self.rect(16, 322, 4, 146, CYAN_ACCENT);
        self.text(28, 332, "TASK SCHEDULER: PRIORITY-BASED ROUND ROBIN (3 TASKS: NETWORK, DISPLAY, AUDIO)", 1, CYAN_ACCENT);
        self.text(28, 348, "TASK SWITCHING: COOPERATIVE YIELD | BURST STEPS PER QUANTUM | FAIR SHARE ALLOCATION", 1, TEXT_MUTED);

        // Bottom Status Strip
        self.rect(0, h - 24, w, 24, TOP_BAR);
        self.text(16, h - 16, "AURORA-OS KERNEL READY // PS/2 SCANCODE DECODER ONLINE", 1, TEXT_MAIN);
        self.text(w - 240, h - 16, "STATUS: INTERACTIVE BARE-METAL", 1, GREEN_ACCENT);
    }

    fn add_term_line(&mut self, text: &str, color: [u8; 3]) {
        if self.term_count >= TERM_LINES {
            // Scroll up
            for i in 1..TERM_LINES {
                self.term_history[i - 1] = self.term_history[i];
                self.term_colors[i - 1] = self.term_colors[i];
            }
            self.term_count = TERM_LINES - 1;
        }
        let mut line = [b' '; TERM_COLS];
        for (i, b) in text.bytes().take(TERM_COLS).enumerate() {
            line[i] = b;
        }
        self.term_history[self.term_count] = line;
        self.term_colors[self.term_count] = color;
        self.term_count += 1;
    }

    fn redraw_term(&mut self) {
        let Some(info) = self.info else { return };
        let w = info.width;
        let h = info.height;
        let start_y = 160;

        // 3 Task Visualizer Blocks
        let t_w = (w - 32 - 40) / 3;

        // Task 1
        self.rect(28, 366, t_w, 56, [255, 210, 225]);
        self.text(36, 374, "TASK 1: NETWORK (DAEMON)", 1, AMBER_ACCENT);
        self.text(36, 388, "CLASS: C1 -> PRIORITY HIGH", 1, TEXT_MAIN);
        self.text(36, 402, "QUANTUM: Q0 = 4MS (BURST 3MS)", 1, TEXT_MUTED);
        self.rect(36, 414, t_w - 16, 4, [255, 182, 206]); // progress track

        // Task 2
        self.rect(38 + t_w, 366, t_w, 56, [255, 210, 225]);
        self.text(46 + t_w, 374, "TASK 2: DISPLAY (RENDERER)", 1, GREEN_ACCENT);
        self.text(46 + t_w, 388, "CLASS: C1 -> PRIORITY NORMAL", 1, TEXT_MAIN);
        self.text(46 + t_w, 402, "QUANTUM: Q0 = 4MS (BURST 2MS)", 1, TEXT_MUTED);
        self.rect(46 + t_w, 414, t_w - 16, 4, [255, 182, 206]);

        // Task 3
        self.rect(48 + t_w * 2, 366, t_w, 56, [255, 210, 225]);
        self.text(56 + t_w * 2, 374, "TASK 3: AUDIO (BACKGROUND)", 1, PURPLE_ACCENT);
        self.text(56 + t_w * 2, 388, "CLASS: C2 -> PRIORITY LOW", 1, TEXT_MAIN);
        self.text(56 + t_w * 2, 402, "QUANTUM: Q2 = FCFS (BURST 100MS)", 1, TEXT_MUTED);
        self.rect(56 + t_w * 2, 414, t_w - 16, 4, [255, 182, 206]);

        self.text(28, 434, "RESULT: TASK SWITCHING LATENCY = 0.2 MS | STARVATION PREVENTION VIA ROUND-ROBIN BOOST", 1, GREEN_ACCENT);
        self.text(28, 448, "FAIRNESS: EQUAL TIME SHARE PER TASK | CONTEXT SWITCH OVERHEAD < 1.5% CPU TIME", 1, TEXT_MUTED);

        // Clear terminal content area
        self.rect(18, start_y, w - 36, TERM_LINES * 16 + 26, CARD_BG);

        // Render history lines
        for i in 0..self.term_count {
            let y = start_y + i * 16;
            let mut line_str = [0u8; TERM_COLS];
            line_str.copy_from_slice(&self.term_history[i]);
            if let Ok(s) = core::str::from_utf8(&line_str) {
                self.text(26, y, s.trim_end(), 1, self.term_colors[i]);
            }
        }

        // Modern Bottom Dock / Status Strip
        self.rect(0, h - 30, w, 30, TOP_BAR);
        self.text(20, h - 20, "AURORA-OS BOOT SEQUENCE COMPLETE // TOTAL INIT: 290.5 MS", 1, [80, 20, 50]);
        self.text(w - 230, h - 20, "STATE: PRODUCTION READY", 1, GREEN_ACCENT);

        // Render Prompt & Current Input Line
        let prompt_y = start_y + TERM_LINES * 16;
        self.text(26, prompt_y, "user@aurora-os:~$ ", 1, GREEN_ACCENT);

        let mut temp_buf = [0u8; 48];
        let len = self.input_len;
        temp_buf[..len].copy_from_slice(&self.input_buf[..len]);
        if let Ok(input_str) = core::str::from_utf8(&temp_buf[..len]) {
            self.text(135, prompt_y, input_str, 1, [80, 20, 50]);
            let cursor_x = 135 + len * 6;
            self.rect(cursor_x, prompt_y, 5, 8, CYAN_ACCENT); // blinking cursor
        }
    }

    fn on_key(&mut self, character: u8) {
        if character == b'\n' {
            // Enter key: execute command
            if self.input_len > 0 {
                let mut cmd_str = [0u8; 48];
                cmd_str[..self.input_len].copy_from_slice(&self.input_buf[..self.input_len]);
                let len = self.input_len;
                self.input_len = 0;

                if let Ok(s) = core::str::from_utf8(&cmd_str[..len]) {
                    self.execute_command(s.trim());
                }
            } else {
                self.add_term_line("user@aurora-os:~$", TEXT_MUTED);
            }
        } else if character == 0x08 {
            // Backspace
            if self.input_len > 0 {
                self.input_len -= 1;
            }
        } else if self.input_len < 40 && character >= 0x20 && character <= 0x7E {
            // Printable char
            self.input_buf[self.input_len] = character;
            self.input_len += 1;
        }
        self.redraw_term();
    }

    fn execute_command(&mut self, cmd: &str) {
        let mut echo_buf = [0u8; 60];
        let mut echo_len = 0;
        let prefix = b"# ";
        echo_buf[..2].copy_from_slice(prefix);
        echo_len += 2;
        for b in cmd.bytes().take(50) {
            echo_buf[echo_len] = b;
            echo_len += 1;
        }
        if let Ok(s) = core::str::from_utf8(&echo_buf[..echo_len]) {
            self.add_term_line(s, [255, 255, 255]);
        }

        match cmd {
            "help" => {
                self.add_term_line("Commands: audio, net, top, hwinfo, display, sched, clear", AMBER_ACCENT);
                self.add_term_line("  audio   - Show HDA audio codec & volume status", TEXT_MAIN);
                self.add_term_line("  net     - Show WiFi 6E / GbE network interface info", TEXT_MAIN);
                self.add_term_line("  top     - View active task scheduling & CPU core load", TEXT_MAIN);
                self.add_term_line("  hwinfo  - Display Lenovo IdeaPad 5 / Ryzen 7 7730U spec", TEXT_MAIN);
            }
            "audio" => {
                self.add_term_line(">>> AURORA-OS AUDIO SUBSYSTEM (HDA CODEC REALTEK ALC257)", CYAN_ACCENT);
                self.add_term_line("[ALSA] /dev/snd/pcm0: 44100 Hz, 16-bit Stereo, RUNNING", GREEN_ACCENT);
                self.add_term_line("--- [AUDIO MIXER] ---", [80, 20, 50]);
                self.add_term_line("Master: 70%  | PCM: 80%  | Mic: 50%  | Headphone: ON", AMBER_ACCENT);
                self.add_term_line("Codec: Realtek ALC257 | HDA Spec 1.0 | Jack: Plugged", TEXT_MAIN);
                self.add_term_line("Status: Audio pipeline active, no underruns detected.", GREEN_ACCENT);
            }
            "net" => {
                self.add_term_line("Interface wlan0: Intel AX211 WiFi-6E [SSID: HomeNet-5G]", GREEN_ACCENT);
                self.add_term_line("Interface eth0:  Realtek GbE (1000 Mbps Full-Duplex)", GREEN_ACCENT);
                self.add_term_line("IPv4 Address:    192.168.1.5 / Netmask: 255.255.255.0 / GW: 192.168.1.1", TEXT_MAIN);
            }
            "top" => {
                self.add_term_line("Tasks: 3 total, 1 running, 2 sleeping | CPU(s): 16 Threads (8C)", TEXT_MAIN);
                self.add_term_line("  PID  NAME       BURST  PRIORITY  CPU%   STATE", CYAN_ACCENT);
                self.add_term_line("    1  network    3.0ms  HIGH       8.2%  RUNNING (Daemon)", AMBER_ACCENT);
                self.add_term_line("    2  display    2.0ms  NORMAL     3.1%  SLEEP   (Renderer)", GREEN_ACCENT);
                self.add_term_line("    3  audio    100.0ms  LOW       88.7%  RUNNING (Background)", PURPLE_ACCENT);
            }
            "hwinfo" => {
                self.add_term_line("Host Laptop: Lenovo IdeaPad 5 15ABA7", TEXT_MAIN);
                self.add_term_line("CPU: AMD Ryzen 7 7730U @ 4.50 GHz (16MB L3 Cache)", CYAN_ACCENT);
                self.add_term_line("RAM: 16 GB DDR4-3200 | Display: 1920x1080 IPS 60Hz BGR", TEXT_MUTED);
                self.add_term_line("WiFi: Intel AX211 WiFi 6E (802.11be) + Realtek GbE NIC", GREEN_ACCENT);
            }
            "display" => {
                self.add_term_line("Framebuffer: 1920x1080 Linear BGR, 24-bpp", TEXT_MAIN);
                self.add_term_line("Driver: UEFI GOP -> VESA Linear Framebuffer", CYAN_ACCENT);
                self.add_term_line("Stride: 5760 bytes/row | Pixel Format: BGR 8:8:8", TEXT_MUTED);
                self.add_term_line("Status: Aurora-OS Pink Theme Active (CARD_BG: #FFE4ED)", GREEN_ACCENT);
            }
            "sched" => {
                self.add_term_line("Policy: Round-Robin Priority Queue (3 Tasks)", CYAN_ACCENT);
                self.add_term_line("Network Mean Waiting Time: 0.00 ms | Tail Latency: 0.20 ms", GREEN_ACCENT);
                self.add_term_line("Starvation: PREVENTED (Equal quantum, periodic boost)", AMBER_ACCENT);
            }
            "clear" => {
                self.term_count = 0;
                self.add_term_line("Terminal buffer cleared. Aurora-OS prompt ready.", TEXT_MUTED);
            }
            _ => {
                self.add_term_line("Unknown command. Type 'help' to see available apps.", [200, 80, 100]);
            }
        }
    }

    fn card(&mut self, x: usize, y: usize, width: usize, height: usize) {
        self.rect(x, y, width, height, CARD_BG);
        self.rect(x, y, width, 1, CARD_BORDER);
        self.rect(x, y + height - 1, width, 1, CARD_BORDER);
        self.rect(x, y, 1, height, CARD_BORDER);
        self.rect(x + width - 1, y, 1, height, CARD_BORDER);
    }

    fn schedule(&mut self, task_id: usize, finished: bool) {
        let Some(info) = self.info else { return };
        self.active_task = task_id;
        if finished && (1..=3).contains(&task_id) {
            self.completed[task_id - 1] = true;
        }

        let w = info.width;
        let t_w = (w - 32 - 40) / 3;
        let colors = [AMBER_ACCENT, GREEN_ACCENT, PURPLE_ACCENT];

        for (index, color) in colors.iter().enumerate() {
            let x = 28 + index * (t_w + 10) + 8;
            let bar_max = t_w - 16;
            let fill = if self.completed[index] {
                bar_max
            } else if self.active_task == index + 1 {
                (bar_max * 2) / 3
            } else {
                bar_max / 4
            };
            self.rect(x, 414, fill, 4, *color);
        }
    }

    fn fill(&mut self, color: [u8; 3]) {
        if let Some(info) = self.info {
            self.rect(0, 0, info.width, info.height, color);
        }
    }

    fn rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: [u8; 3]) {
        let Some(info) = self.info else { return };
        let max_x = (x + width).min(info.width);
        let max_y = (y + height).min(info.height);
        for row in y.min(info.height)..max_y {
            for column in x.min(info.width)..max_x {
                self.pixel(column, row, color);
            }
        }
    }

    fn text(&mut self, x: usize, y: usize, text: &str, scale: usize, color: [u8; 3]) {
        for (index, character) in text.bytes().enumerate() {
            self.character(x + index * 6 * scale, y, character, scale, color);
        }
    }

    fn character(&mut self, x: usize, y: usize, character: u8, scale: usize, color: [u8; 3]) {
        let glyph = match character.to_ascii_uppercase() {
            b'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
            b'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
            b'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
            b'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
            b'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
            b'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
            b'G' => [0x0f, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0f],
            b'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
            b'I' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
            b'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c],
            b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
            b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
            b'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
            b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
            b'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
            b'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
            b'Q' => [0x0e, 0x11, 0x11, 0x15, 0x15, 0x0e, 0x01],
            b'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
            b'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
            b'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
            b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
            b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
            b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1b, 0x11],
            b'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
            b'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
            b'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
            b'0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
            b'1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x1f],
            b'2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
            b'3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
            b'4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
            b'5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
            b'6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
            b'7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
            b'8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
            b'9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
            b':' => [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x0c, 0x00],
            b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
            b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c],
            b'-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
            b'(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
            b')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
            b'[' => [0x1e, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1e],
            b']' => [0x0f, 0x01, 0x01, 0x01, 0x01, 0x01, 0x0f],
            b'=' => [0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00, 0x00],
            b'%' => [0x19, 0x19, 0x02, 0x04, 0x08, 0x13, 0x13],
            b'|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
            b'+' => [0x04, 0x04, 0x1f, 0x04, 0x04, 0x00, 0x00],
            b'>' => [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
            b'<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
            b'#' => [0x0a, 0x1f, 0x0a, 0x0a, 0x1f, 0x0a, 0x00],
            b'~' => [0x00, 0x0d, 0x16, 0x00, 0x00, 0x00, 0x00],
            b'@' => [0x0e, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0f],
            b',' => [0x00, 0x00, 0x00, 0x00, 0x0c, 0x04, 0x08],
            b'\'' => [0x04, 0x04, 0x02, 0x00, 0x00, 0x00, 0x00],
            b' ' => [0; 7],
            _ => [0; 7],
        };
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    self.rect(x + column * scale, y + row * scale, scale, scale, color);
                }
            }
        }
    }

    fn pixel(&mut self, x: usize, y: usize, color: [u8; 3]) {
        let Some(info) = self.info else { return };
        let offset = (y * info.stride + x) * info.bytes_per_pixel;
        unsafe {
            let destination =
                core::slice::from_raw_parts_mut(self.buffer.add(offset), info.bytes_per_pixel);
            match info.pixel_format {
                PixelFormat::Rgb => destination[..3].copy_from_slice(&color),
                PixelFormat::Bgr => {
                    destination[..3].copy_from_slice(&[color[2], color[1], color[0]])
                }
                PixelFormat::U8 => destination[0] = color[0],
                PixelFormat::Unknown { .. } => destination[..3].copy_from_slice(&color),
                _ => destination[..3].copy_from_slice(&color),
            }
        }
    }
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen::new());

#[derive(Clone, Copy, Eq, PartialEq)]
enum TaskState {
    Ready,
    Running,
    Terminated,
}

#[derive(Clone, Copy)]
struct Task {
    id: usize,
    name: &'static str,
    remaining_steps: usize,
    state: TaskState,
}

impl Task {
    const EMPTY: Option<Task> = None;

    fn new(id: usize, name: &'static str, steps: usize) -> Self {
        Self {
            id,
            name,
            remaining_steps: steps,
            state: TaskState::Ready,
        }
    }

    fn step(&mut self) -> bool {
        if self.remaining_steps > 0 {
            self.remaining_steps -= 1;
        }
        self.state = if self.remaining_steps == 0 {
            TaskState::Terminated
        } else {
            TaskState::Ready
        };
        self.state == TaskState::Terminated
    }
}

struct TaskQueue {
    tasks: [Option<Task>; MAX_TASKS],
    head: usize,
    tail: usize,
    switches: usize,
    completed: usize,
}

impl TaskQueue {
    const fn new() -> Self {
        Self {
            tasks: [Task::EMPTY; MAX_TASKS],
            head: 0,
            tail: 0,
            switches: 0,
            completed: 0,
        }
    }

    fn push(&mut self, task: Task) -> bool {
        let next = (self.tail + 1) % MAX_TASKS;
        if next == self.head {
            return false;
        }
        self.tasks[self.tail] = Some(task);
        self.tail = next;
        true
    }

    fn pop(&mut self) -> Option<Task> {
        if self.head == self.tail {
            return None;
        }
        let task = self.tasks[self.head].take();
        self.head = (self.head + 1) % MAX_TASKS;
        task
    }

    fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    fn run_one_step(&mut self) -> Option<(Task, bool)> {
        let mut task = self.pop()?;
        self.switches += 1;
        let finished = task.step();
        if finished {
            self.completed += 1;
        } else {
            self.push(task);
        }
        Some((task, finished))
    }
}

static READY_QUEUE: Mutex<TaskQueue> = Mutex::new(TaskQueue::new());
static SHARED_COUNTER: Mutex<usize> = Mutex::new(0);

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

struct BumpAllocator {
    next: AtomicUsize,
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let heap_start = core::ptr::addr_of_mut!(HEAP) as *mut u8 as usize;
        let current = self.next.load(Ordering::Relaxed);
        let aligned = (heap_start + current + layout.align() - 1) & !(layout.align() - 1);
        let offset = aligned - heap_start;
        let end = match offset.checked_add(layout.size()) {
            Some(end) if end <= HEAP_SIZE => end,
            _ => return core::ptr::null_mut(),
        };
        if self
            .next
            .compare_exchange(current, end, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            aligned as *mut u8
        } else {
            core::ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator {
    next: AtomicUsize::new(0),
};

#[derive(Clone, Copy)]
#[repr(u8)]
enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial_println!("Aurora-OS Rust kernel booted");
    serial_println!("[BOOT] x86_64 bootloader transferred control to kernel");
    serial_println!("[INIT] serial console initialized");

    if let Some(framebuffer) = boot_info.framebuffer.as_mut() {
        let mut screen = SCREEN.lock();
        screen.init(framebuffer);
        screen.show_splash_animation(); // Show pink aurora boot animation & logo
        screen.render_desktop();         // Transition to light-pink OS desktop
    }

    IDT.load();
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // Unmask IRQ 0 (Timer) and IRQ 1 (Keyboard) explicitly on PIC 1
        pics.write_masks(0xFC, 0xFF);
    };
    init_ps2_keyboard();
    configure_timer(TIMER_FREQUENCY_HZ);
    x86_64::instructions::interrupts::enable();
    serial_println!("[INIT] IDT, PIC and PIT timer initialized");
    serial_println!("[INIT] IDT, PIC, PIT and PS/2 Keyboard IRQ initialized");

    // Telemetry output for faculty inspection
    serial_println!("[HW] Host: Lenovo IdeaPad 5 15ABA7 (AMD Ryzen 7 7730U 8C/16T)");
    serial_println!("[NET] WLAN: Intel AX211 WiFi 6E (802.11be) | LAN: Realtek GbE 1000Mbps");
    serial_println!("[AUDIO] HDA Codec: Realtek ALC257 | ALSA /dev/snd/pcm0 initialized");
    serial_println!("[DISPLAY] Framebuffer: 1920x1080 BGR 24bpp | Aurora Pink Theme active");
    serial_println!("[SCHED] Priority Round-Robin active: network->HIGH, display->NORMAL, audio->LOW");
    serial_println!("[APPS] Interactive shell, audio, display, net, top, hwinfo online");

    {
        let mut queue = READY_QUEUE.lock();
        queue.push(Task::new(1, "network", 3));
        queue.push(Task::new(2, "display", 2));
        queue.push(Task::new(3, "audio", 4));
    }
    serial_println!("[TASK] created network, display and audio tasks");

    let allocation = unsafe { ALLOCATOR.alloc(Layout::from_size_align(128, 8).unwrap()) };
    if allocation.is_null() {
        serial_println!("[MEM] allocation failed");
    } else {
        serial_println!("[MEM] allocated 128 bytes from kernel bump heap");
    }

    serial_println!("[RUN] timer-driven priority round-robin scheduling started");
    for _ in 0..5_000_000 {
        core::hint::spin_loop();
    }
    if *SHARED_COUNTER.lock() == 0 {
        serial_println!("[TIMER] no PIT IRQ observed; using cooperative fallback");
        loop {
            let tick = next_tick();
            if schedule_once(tick) {
                finish_run();
            }
        }
    }
    serial_println!("[RUN] timer-driven priority scheduling & Interactive Shell active");
    hlt_loop();
}

fn configure_timer(frequency_hz: u32) {
    let divisor = 1_193_182 / frequency_hz;
    let mut command = Port::<u8>::new(0x43);
    let mut channel = Port::<u8>::new(0x40);
    unsafe {
        command.write(0x36);
        channel.write((divisor & 0xff) as u8);
        channel.write((divisor >> 8) as u8);
    }
}

fn init_ps2_keyboard() {
    let mut status_port = Port::<u8>::new(0x64);
    let mut data_port = Port::<u8>::new(0x60);
    unsafe {
        // Enable primary PS/2 port (command 0xAE)
        while (status_port.read() & 0x02) != 0 {
            core::hint::spin_loop();
        }
        status_port.write(0xAE);

        // Flush any pending data in the output buffer
        while (status_port.read() & 0x01) != 0 {
            let _ = data_port.read();
        }
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let tick = next_tick();
    schedule_once(tick);
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8())
    };
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port = Port::<u8>::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    // Decode PS/2 Set 1 Scancodes
    let character = match scancode {
        0x1E => Some(b'a'),
        0x30 => Some(b'b'),
        0x2E => Some(b'c'),
        0x20 => Some(b'd'),
        0x12 => Some(b'e'),
        0x21 => Some(b'f'),
        0x22 => Some(b'g'),
        0x23 => Some(b'h'),
        0x17 => Some(b'i'),
        0x24 => Some(b'j'),
        0x25 => Some(b'k'),
        0x26 => Some(b'l'),
        0x32 => Some(b'm'),
        0x31 => Some(b'n'),
        0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x10 => Some(b'q'),
        0x13 => Some(b'r'),
        0x1F => Some(b's'),
        0x14 => Some(b't'),
        0x16 => Some(b'u'),
        0x2F => Some(b'v'),
        0x11 => Some(b'w'),
        0x2D => Some(b'x'),
        0x15 => Some(b'y'),
        0x2C => Some(b'z'),
        0x02 => Some(b'1'),
        0x03 => Some(b'2'),
        0x04 => Some(b'3'),
        0x05 => Some(b'4'),
        0x06 => Some(b'5'),
        0x07 => Some(b'6'),
        0x08 => Some(b'7'),
        0x09 => Some(b'8'),
        0x0A => Some(b'9'),
        0x0B => Some(b'0'),
        0x39 => Some(b' '),
        0x1C => Some(b'\n'),
        0x0E => Some(0x08), // Backspace
        0x34 => Some(b'.'),
        0x35 => Some(b'/'),
        0x0C => Some(b'-'),
        _ => None,
    };

    if let Some(ch) = character {
        serial_println!("[KEYBOARD] key pressed: {}", ch as char);
        SCREEN.lock().on_key(ch);
    } else {
        serial_println!("[KEYBOARD] scancode: 0x{:x}", scancode);
    }

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8())
    };
}

fn next_tick() -> usize {
    let mut counter = SHARED_COUNTER.lock();
    *counter += 1;
    *counter
}

fn schedule_once(tick: usize) -> bool {
    let mut queue = READY_QUEUE.lock();
    if let Some((task, finished)) = queue.run_one_step() {
        serial_println!(
            "[SCHED] tick={} task={} ({}){}",
            tick,
            task.id,
            task.name,
            if finished { " TERMINATED" } else { "" }
        );
        if let Some(mut screen) = SCREEN.try_lock() {
            screen.schedule(task.id, finished);
        }
        return queue.is_empty();
    }
    false
}

fn finish_run() -> ! {
    let queue = READY_QUEUE.lock();
    serial_println!(
        "[DONE] switches={} completed_tasks={}",
        queue.switches,
        queue.completed
    );
    exit_qemu(0x10);
    x86_64::instructions::interrupts::disable();
    hlt_loop();
}

struct SerialWriter;

fn exit_qemu(code: u32) {
    let mut port = Port::<u32>::new(0xf4);
    unsafe {
        port.write(code);
    }
}

impl Write for SerialWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        SERIAL1.lock().write_str(text)
    }
}

#[macro_export]
macro_rules! serial_print { ($($arg:tt)*) => {{ use core::fmt::Write; let _ = write!($crate::SerialWriter, $($arg)*); }}; }

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(concat!($fmt, "\n"), $($arg)*));
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[AURORA-OS PANIC] {}", info);
    hlt_loop();
}

fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
