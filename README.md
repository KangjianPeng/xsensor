# XSensor

XSensor is a desktop application built with Rust and Dioxus for monitoring and configuring BLE-based sensor devices.

## Features

- **BLE Connectivity**:
  - Automatic scanning for compatible devices.
  - Stable scan results that can be selected manually while scanning.
  - Real-time connection status and signal strength (RSSI) monitoring.

- **Real-time Status**:
  - Visualizes sensor data streams.
  - Reads PS values from the XSensor profile and follows its configured refresh frequency (0–15 Hz, where 0 stops periodic updates).
  - Reads and subscribes to the standard BLE Battery Service.
  - **Smart Counting**: Implements logic to filter signal bounce and release events, ensuring accurate event counting.

- **Parameter Configuration**:
  - Read and write device parameters wirelessly.
  - Configures the low and high PS thresholds and the UTF-8 BLE device name (up to 20 bytes).

## Tech Stack

- **Language**: [Rust](https://www.rust-lang.org/)
- **UI Framework**: [Dioxus](https://dioxuslabs.com/) (v0.7)
- **Styling**: [Tailwind CSS](https://tailwindcss.com/)
- **BLE Library**: [btleplug](https://github.com/deviceplug/btleplug)

## Project Structure

```
xsensor/
├── assets/          # Static assets and compiled CSS
├── src/
│   ├── api/         # BLE implementation and data models
│   ├── components/  # Reusable UI components
│   ├── views/       # Application pages (Connection, Status, Parameters)
│   ├── context.rs   # Global application state management
│   └── main.rs      # Application entry point
├── Cargo.toml       # Rust dependencies
└── Dioxus.toml      # Dioxus configuration
```

## Getting Started

### Prerequisites

- Rust and Cargo installed.
- Dioxus CLI installed (`cargo install dioxus-cli`).

### Running the App

```bash
dx serve --platform desktop
```

## Development Notes

- **Connection flow**: Scanning only discovers devices. Start a connection by clicking the device row's connection button.
- **Counter Logic**: The status view ignores `0` values from the notification stream to prevent double-counting on button release.
