//! The devices a prototype is shown on: a name, the size of the screen in
//! points and the colours the model comes in.
//!
//! The table is FigCraft's (`crates/doc/src/proto.rs`, `PrototypeSettings::DEVICES`,
//! MIT OR Apache-2.0; see `NOTICE`), with the two entries a page can also pick
//! that are not models: `Custom`, a screen of the page's own `customSize`
//! (FigCraft calls it "Custom size"), and `None`, which shows a board at its
//! own size.

/// A colour a device comes in: its body, with the glass around the screen
/// staying black.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceColor {
    pub name: &'static str,
    pub rgb: [u8; 3],
}

const fn c(name: &'static str, r: u8, g: u8, b: u8) -> DeviceColor {
    DeviceColor {
        name,
        rgb: [r, g, b],
    }
}

/// A device, with its screen held upright.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Device {
    pub name: &'static str,
    /// The screen's width in points; 0 for `Custom` and `None`, which have
    /// no screen of their own.
    pub width: f64,
    /// The screen's height in points; 0 for `Custom` and `None`.
    pub height: f64,
    /// Whether it can be turned to landscape: phones and tablets can, laptops
    /// and desktops cannot.
    pub rotates: bool,
    /// The colours it comes in, the default first; none for `Custom` and
    /// `None`.
    pub colors: &'static [DeviceColor],
}

/// The name of the device whose screen is the page's `customSize`.
pub const CUSTOM: &str = "Custom";

/// The name of the choice that shows a board at its own size.
pub const NONE: &str = "None";

/// The devices to pick from.
pub const DEVICES: [Device; 21] = [
    Device {
        name: "iPhone 17",
        width: 402.0,
        height: 874.0,
        rotates: true,
        colors: &[c("Black", 0x38, 0x39, 0x3D), c("White", 0xE9, 0xE9, 0xE5)],
    },
    Device {
        name: "iPhone 17 Pro",
        width: 402.0,
        height: 874.0,
        rotates: true,
        colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 17 Pro Max",
        width: 440.0,
        height: 956.0,
        rotates: true,
        colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone Air",
        width: 420.0,
        height: 912.0,
        rotates: true,
        colors: &[c("Black", 0x38, 0x39, 0x3D), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 16",
        width: 393.0,
        height: 852.0,
        rotates: true,
        colors: &[
            c("Black", 0x3A, 0x3B, 0x3F),
            c("White", 0xEE, 0xED, 0xE8),
            c("Pink", 0xEC, 0xBF, 0xCB),
            c("Teal", 0xA3, 0xCF, 0xC6),
            c("Blue", 0x72, 0x84, 0xD4),
        ],
    },
    Device {
        name: "iPhone 16 Pro",
        width: 402.0,
        height: 874.0,
        rotates: true,
        colors: &[c("Black", 0x3B, 0x3A, 0x38), c("White", 0xEC, 0xEB, 0xE6)],
    },
    Device {
        name: "iPhone 16 Pro Max",
        width: 440.0,
        height: 956.0,
        rotates: true,
        colors: &[
            c("Black", 0x3B, 0x3A, 0x38),
            c("White", 0xEC, 0xEB, 0xE6),
            c("Natural", 0xBF, 0xB9, 0xAF),
            c("Sand", 0xC6, 0xAC, 0x8F),
        ],
    },
    Device {
        name: "iPhone 16 Plus",
        width: 430.0,
        height: 932.0,
        rotates: true,
        colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
    },
    Device {
        name: "iPhone 15 Pro",
        width: 393.0,
        height: 852.0,
        rotates: true,
        colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 15 Pro Max",
        width: 430.0,
        height: 932.0,
        rotates: true,
        colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 15 Plus",
        width: 430.0,
        height: 932.0,
        rotates: true,
        colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
    },
    Device {
        name: "iPhone 14 Pro",
        width: 393.0,
        height: 852.0,
        rotates: true,
        colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 14 Pro Max",
        width: 430.0,
        height: 932.0,
        rotates: true,
        colors: &[c("Black", 0x3B, 0x3A, 0x38), c("Silver", 0xD7, 0xD7, 0xD5)],
    },
    Device {
        name: "iPhone 14",
        width: 390.0,
        height: 844.0,
        rotates: true,
        colors: &[c("Black", 0x3A, 0x3B, 0x3F), c("White", 0xEE, 0xED, 0xE8)],
    },
    Device {
        name: "Android Compact",
        width: 412.0,
        height: 917.0,
        rotates: true,
        colors: &[
            c("Black", 0x2E, 0x2F, 0x33),
            c("Silver", 0xD6, 0xD8, 0xDB),
            c("Green", 0xA6, 0xC0, 0xA3),
        ],
    },
    Device {
        name: "Android Medium",
        width: 700.0,
        height: 840.0,
        rotates: true,
        colors: &[c("Black", 0x2E, 0x2F, 0x33), c("Silver", 0xD6, 0xD8, 0xDB)],
    },
    Device {
        name: "iPad mini",
        width: 744.0,
        height: 1133.0,
        rotates: true,
        colors: &[
            c("Grey", 0x6A, 0x6B, 0x6F),
            c("Silver", 0xE0, 0xE1, 0xE3),
            c("Blue", 0x9C, 0xB1, 0xC9),
            c("Purple", 0xC6, 0xBC, 0xD7),
        ],
    },
    Device {
        name: "MacBook Air",
        width: 1280.0,
        height: 832.0,
        rotates: false,
        colors: &[
            c("Silver", 0xC9, 0xCA, 0xCE),
            c("Grey", 0x7B, 0x7C, 0x80),
            c("Dark blue", 0x2E, 0x33, 0x3D),
            c("Cream", 0xE0, 0xD9, 0xCB),
        ],
    },
    Device {
        name: "Desktop",
        width: 1440.0,
        height: 1024.0,
        rotates: false,
        colors: &[c("Silver", 0xB8, 0xB9, 0xBD), c("Black", 0x2C, 0x2C, 0x2E)],
    },
    Device {
        name: CUSTOM,
        width: 0.0,
        height: 0.0,
        rotates: false,
        colors: &[],
    },
    Device {
        name: NONE,
        width: 0.0,
        height: 0.0,
        rotates: false,
        colors: &[],
    },
];

/// The device named `name`, or `None` for a name the table lacks.
pub fn device(name: &str) -> Option<&'static Device> {
    DEVICES.iter().find(|device| device.name == name)
}
