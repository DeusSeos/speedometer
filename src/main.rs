use std::{char, time::Duration};

use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{ascii::{FONT_10X20, FONT_6X10, FONT_6X13_BOLD}, iso_8859_15::FONT_10X20, MonoTextStyle},
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

// Replace `Display` with the appropriate type for your specific display
fn draw_speedometer<Display>(display: &mut Display, speed: f32) -> Result<(), Display::Error>
where
    Display: DrawTarget<Color = Rgb565>,
{
    // Center of the speedometer
    let center = Point::new(119, 119);
    let radius = 112;
    let circle_offset = Point::new(1, 1);
    let top_left = center - Point::new(radius, radius) + circle_offset;
    let neon_green = Rgb565::new(0, 191, 83);

    // Draw the dial
    Circle::new(top_left, radius as u32 * 2)
        .into_styled(PrimitiveStyle::with_stroke(neon_green, 4))
        .draw(display)?;

    
    // 0 is right side of the display and 180 is left side of the display with the notch at the top.
    let start_angle = std::f32::consts::PI;
    let tick_length = 20;
    let text_style = MonoTextStyle::new(&FONT_6X13_BOLD, Rgb565::WHITE);
    let unit_text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let speed_text_style = MonoTextStyle::new(&PROFONT_24_POINT, Rgb565::WHITE);

    // Draw speed markings and numbers
    for i in 0..=12 {
        let angle = (i as f32 * 2.0 * std::f32::consts::PI / 24.0) + start_angle;
        // let angle = start_angle + std::f32::consts::PI - (i as f32 * 2.0 * std::f32::consts::PI / 24.0);
        let outer_end = center
            + Point::new(
                (angle.cos() * radius as f32) as i32,
                (angle.sin() * radius as f32) as i32,
            );
        let inner_end = center
            + Point::new(
                (angle.cos() * (radius - tick_length) as f32) as i32,
                (angle.sin() * (radius - tick_length) as f32) as i32,
            );

        // Draw markings
        Line::new(outer_end, inner_end)
            .into_styled(PrimitiveStyle::with_stroke(
                neon_green,
                if i % 2 == 0 { 3 } else { 1 },
            ))
            .draw(display)?;
        
        let default_text_radius = 15;

        // Draw numbers at every 2nd marking
        if i % 2 == 0 {
            let number = i * 10;
            let character_size = text_style.font.character_size;
            let number_width = if number < 100 { character_size.width * 2 } else { character_size.width * 3 };
            let text_offset = Point::new((number_width / 2) as i32, (character_size.height / 2) as i32);
            let additional_offset = Point::new(1, 9); // Your adjusted offset
            let text_angle = angle + start_angle;
            let text_pos = center - Point::new((text_angle.cos() * (radius - (default_text_radius+tick_length)) as f32) as i32, (text_angle.sin() * (radius - (default_text_radius+tick_length)) as f32) as i32) - text_offset + additional_offset;
            Text::new( &format!("{:2}", number), text_pos, text_style, ) .draw(display)?; } }

    // Calculate needle position based on speed
    let angle = speed_to_angle(speed, start_angle);
    let needle_end = center
        + Point::new(
            (angle.cos() * (radius - 20) as f32) as i32,
            (angle.sin() * (radius - 20) as f32) as i32,
        );

    // Draw the needle
    Line::new(center, needle_end)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::RED, 2))
        .draw(display)?;

    
    // Display speed as text
    let speed_text = format!("{:3}", speed);
    let character_size = speed_text_style.font.character_size;
    let text_width = speed_text.len() as i32 * character_size.width as i32;
    let text_offset = Point::new((text_width / 2) as i32, (character_size.height / 2) as i32);
    let additional_offset = Point::new(1, 40); // Your adjusted offset
    let text_pos = center - text_offset + additional_offset;
    Text::new(&speed_text, text_pos, speed_text_style) .draw(display)?;

    // Display unit as text
    let speed_text = format!("mi/hr");
    let character_size = unit_text_style.font.character_size;
    let text_width = speed_text.len() as i32 * character_size.width as i32;
    let text_offset = Point::new((text_width / 2) as i32, (character_size.height / 2) as i32);
    let additional_offset = Point::new(1, 70); // Your adjusted offset
    let text_pos = center - text_offset + additional_offset;
    Text::new(&speed_text, text_pos, unit_text_style) .draw(display)?;


    Ok(())
}

fn speed_to_angle(speed: f32, start_angle: f32) -> f32 {
    // Convert speed to an angle for the needle 
    // using 960 = 120 * 8 giving us more angles to work with
    ((8.0*speed)/960.0) * std::f32::consts::PI + start_angle
}

fn main() {
    // setup of the SPI
    // Table of GC9A01 driver (https://www.waveshare.com/wiki/1.28inch_LCD_Module) to physical pinout to function to BCM pin (https://pinout.xyz/)
    // GC9A01 | Pi | SPI      | BCM
    //  DIN   | 19 | MOSI     | 10
    //  CLK   | 23 | SCLK     | 11
    let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 5_000_000, Mode::Mode0)
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

    loop {
        display_driver.clear();
        draw_speedometer(&mut display_driver, speed).ok();
        display_driver.flush().ok();
        speed += 1.0;
    }
}
