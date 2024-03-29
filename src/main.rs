use std::{time::{Duration, Instant}, fs, env};
use notify::{RecommendedWatcher, Watcher, RecursiveMode, Config, event::{DataChange, ModifyKind}, EventKind};
use std::sync::mpsc::channel;

use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{ascii::{FONT_10X20, FONT_6X13_BOLD}, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle},
    text::Text,
};
use profont::PROFONT_24_POINT;
use gc9a01::{
    display::DisplayResolution240x240,
    mode::{BufferedGraphics, DisplayConfiguration},
    prelude::SPIInterface,
    rotation::DisplayRotation,
    Gc9a01, SPIDisplayInterface,
};
use rppal::{
    gpio::{Gpio, OutputPin},
    hal::Delay,
    pwm::{self, Pwm},
    spi::*,
};

use anyhow::{anyhow, Result};


fn set_brightness(bl: &mut Pwm, brightness: u8) -> Result<(), anyhow::Error> {
    let pulse_width = match brightness {
        0 => Duration::from_micros(0),
        _ => Duration::from_micros((brightness as u64 * 3_000) / 255),
    };
    match bl.set_pulse_width(pulse_width) {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow!("Error setting pulse width: {}", e)),
    }
}


fn draw_speedometer<Display>(
    display: &mut Display, 
    speed: f32,
    circle: Circle,
    circle_style: PrimitiveStyle<Rgb565>,
    text_style: MonoTextStyle<'_, Rgb565>, 
    speed_text_style: MonoTextStyle<'_, Rgb565>, 
    unit_text_style: MonoTextStyle<'_, Rgb565>
) -> Result<(), Display::Error>
where
    Display: DrawTarget<Color = Rgb565>,
{
    // let function_now = Instant::now();
    // let mut now = Instant::now();

    // Constants and precomputed values
    const PI: f32 = std::f32::consts::PI;
    const TICK_LENGTH: i32 = 20;
    const DEFAULT_TEXT_RADIUS: i32 = 15;
    const NEEDLE_LENGTH: u8 = 92; // 112 - 20
    const TEXT_OFFSET_Y: i32 = 40;
    const UNIT_TEXT: &str = "mi/hr";
    const UNIT_TEXT_POS: Point = Point::new(95, 179);
    const START_ANGLE: f32 = std::f32::consts::PI;
    const CENTER: Point = Point::new(119, 119);
    const RADIUS: i32 = 112;
    const DIAMETER: u32 = RADIUS as u32 * 2;
    const TOP_LEFT: Point = Point::new(8, 8);

    // let mut elapsed = now.elapsed();
    // println!("Precompute Elapsed: {:?}", elapsed);

    // now = Instant::now();

    // Draw the dial
    circle.into_styled(circle_style).draw(display)?;

    // elapsed = now.elapsed();
    // println!("Circle Elapsed: {:?}", elapsed);

    // now = Instant::now();

    for i in 0..=12 {
        let angle = (i as f32 * 2.0 * PI / 24.0) + START_ANGLE;
        let outer_end = CENTER + Point::new(
            (angle.cos() * RADIUS as f32) as i32,
            (angle.sin() * RADIUS as f32) as i32,
        );
        let inner_end = CENTER + Point::new(
            (angle.cos() * (RADIUS - TICK_LENGTH) as f32) as i32,
            (angle.sin() * (RADIUS - TICK_LENGTH) as f32) as i32,
        );

        Line::new(outer_end, inner_end)
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(0, 191, 83), if i % 2 == 0 { 3 } else { 1 }))
            .draw(display)?;
        
        if i % 2 == 0 {
            let number = i * 10;
            let number_width = match number {
                1..=9 => 6,
                10..=99 => 12,
                _ => 18,
            };
            let text_offset = Point::new((number_width / 2) as i32, 7); // Half of 13 (height)
            let additional_offset = Point::new(1, 9);
            let text_angle = angle + START_ANGLE;
            let text_pos = CENTER - Point::new(
                (text_angle.cos() * (RADIUS - (DEFAULT_TEXT_RADIUS + TICK_LENGTH)) as f32) as i32, 
                (text_angle.sin() * (RADIUS - (DEFAULT_TEXT_RADIUS + TICK_LENGTH)) as f32) as i32
            ) - text_offset + additional_offset;
            Text::new(&format!("{:2}", number), text_pos, text_style).draw(display)?;
        }
    }

    // elapsed = now.elapsed();
    // println!("Tick Elapsed: {:?}", elapsed);

    

    // now = Instant::now();

    // // Calculate needle position based on speed
    let angle = speed_to_angle(speed, START_ANGLE);
    let needle_end = CENTER + Point::new(
        (angle.cos() * NEEDLE_LENGTH as f32) as i32,
        (angle.sin() * NEEDLE_LENGTH as f32) as i32,
    );

    Line::new(CENTER, needle_end)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::RED, 2))
        .draw(display)?;

    // elapsed = now.elapsed();
    // println!("Needle Elapsed: {:?}", elapsed);

    // now = Instant::now();

    // Display speed as text
    let speed_text = format!("{}", speed);
    let speed_text_width = match speed_text.len() {
        1 => 16,
        2 => 32,
        _ => 48,
    };
    let text_offset = Point::new((speed_text_width / 2) as i32, (speed_text_style.font.character_size.height / 2) as i32);
    let text_pos = CENTER - text_offset + Point::new(1, TEXT_OFFSET_Y);
    
    Text::new(&speed_text, text_pos, speed_text_style).draw(display)?;

    // Display unit as text
    Text::new(UNIT_TEXT, UNIT_TEXT_POS, unit_text_style).draw(display)?;

    // elapsed = now.elapsed();
    // println!("Text Elapsed: {:?}", elapsed);

    // elapsed = function_now.elapsed();
    // println!("Function Elapsed: {:?}", elapsed);

    Ok(())
}

fn speed_to_angle(speed: f32, start_angle: f32) -> f32 {
    ((8.0 * speed) / 960.0) * std::f32::consts::PI + start_angle
}


fn main() {
    // setup of the SPI
    // Table of GC9A01 driver (https://www.waveshare.com/wiki/1.28inch_LCD_Module) to physical pinout to function to BCM pin (https://pinout.xyz/)
    // GC9A01 | Pi | SPI      | BCM
    //  DIN   | 19 | MOSI     | 10
    //  CLK   | 23 | SCLK     | 11
    let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 27_000_000, Mode::Mode0)
        .expect("Error setting SPI preferences");

    //setup the rest of the pins for Gc9a01 driver
    // Note: Slave Select(SS) is also know as Chip Enable(CE) or Chip Select(CS)
    // GC9A01 | Pi | BCM
    //   CS   | 24 | 8 (CE0)
    //   DC   | 22 | 25
    //   RST  | 13 | 27
    //   BL   | 12 | 18
    let gpio = Gpio::new().expect("Could not set up GPIO");

    // CS pin
    let cs = gpio.get(8).expect("Unable to get pin 8 (CS)").into_output();
    // Data or Command? pin (Set which mode to be in 0 for command, 1 for data)
    let dc = gpio.get(25).expect("Unable to get pin 13").into_output();
    // reset pin
    let mut reset = gpio.get(27).expect("Unable to get pin 13").into_output();
    // backlight pin
    // The LEDPWM
    // duty is calculated as DBV[7:0]/255 x period (affected by OSC frequency).
    // For example: LEDPWM period = 3ms, and DBV[7:0] = ‘200DEC’. Then LEDPWM duty = 200 / 255=78.1%.
    // Correspond to the LEDPWM period = 3 ms, the high-level of LEDPWM (high effective) = 2.344ms, and the
    // low-level of LEDPWM = 0.656ms.
    let period = Duration::from_micros(3_000);
    let pulse_width = Duration::from_micros(3_000);

    let mut bl = Pwm::with_period(
        pwm::Channel::Pwm0,
        period,
        pulse_width,
        pwm::Polarity::Normal,
        true,
    )
    .expect("Unable to set up PWM");

    // create the interface for the display
    let interface = SPIDisplayInterface::new(spi, dc, cs);

    let mut display_driver: Gc9a01<
        SPIInterface<Spi, OutputPin, OutputPin>,
        DisplayResolution240x240,
        BufferedGraphics<DisplayResolution240x240>,
    > = Gc9a01::new(
        interface,
        DisplayResolution240x240,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics();

    let mut delay = Delay::new();

    display_driver.reset(&mut reset, &mut delay).ok();
    display_driver.init(&mut delay).ok();

    set_brightness(&mut bl, 255).expect("Unable to set brightness");

    // set speed to 0
    let mut speed = 0.0;

    const NEON_GREEN: Rgb565 = Rgb565::new(0, 191, 83);

    let text_style = MonoTextStyle::new(&FONT_6X13_BOLD, Rgb565::WHITE);
    let speed_text_style = MonoTextStyle::new(&PROFONT_24_POINT, Rgb565::WHITE);
    let unit_text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let circle_style = PrimitiveStyle::with_stroke(NEON_GREEN, 4);
    const RADIUS: i32 = 112;
    const DIAMETER: u32 = RADIUS as u32 * 2;
    const TOP_LEFT: Point = Point::new(8, 8);
    const CIRCLE: Circle = Circle::new(TOP_LEFT, DIAMETER);

    // Create a channel to receive the events.
    let (tx, rx) = channel();

    let path = std::path::Path::new("./data/speed.txt");

    // Create a watcher object, delivering debounced events.
    // The Duration::from_secs(10) is the debounce period.
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).unwrap();

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(&path, RecursiveMode::NonRecursive).unwrap();

    loop {
        match rx.recv() {
            Ok(result) => match result {
                Ok(event) => match event.kind {
                    EventKind::Modify(ModifyKind::Data(DataChange::Any)) => {
                        // println!("data changed");
                        // read the file and update the speed
                        // set speed to 0
                        match fs::read_to_string("./data/speed.txt") {
                            Ok(s) => {
                                // Check if the string is empty
                                if s.is_empty() {
                                    // println!("Speed is empty");
                                    continue;
                                }

                                // println!("Speed: {}", s);
                                speed = s.trim().parse::<f32>().unwrap();
                                // update the display
                                display_driver.clear();
                                draw_speedometer(&mut display_driver, speed, CIRCLE, circle_style, text_style, speed_text_style, unit_text_style).ok();
                                display_driver.flush().ok();
                                // let elapsed = now.elapsed();
                            },
                            Err(e) => println!("Error reading file: {:?}", e),
                        }

                    },
                    _ => continue,
                },

                Err(e) => println!("watch error: {:?}", e),

            },
            Err(e) => println!("watch error: {:?}", e),
    }
}

}

