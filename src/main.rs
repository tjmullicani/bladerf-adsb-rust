use clap::{command, Parser};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::io::{Write};
use std::net::TcpStream;
use std::str::FromStr;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

extern crate log;

extern crate bladerf;
use bladerf::{BladeRFModuleConfig};
use bladerf::bladerf::bladerf_fpga_size::*;
use bladerf::bladerf::bladerf_module::*;
use bladerf::bladerf::bladerf_format::*;
use bladerf::bladerf::bladerf_gain_mode::*;
use bladerf::bladerf::Struct_bladerf_devinfo;

use std::sync::atomic::Ordering;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::{channel, Sender};
use log::{LevelFilter};
use log::{trace, debug, info, warn, error};
use env_logger::Builder;
use std::ffi::CStr;
use thousands::Separable;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
  // Sets the target bandwidth
  #[arg(short = 'b', long = "bandwidth", value_name = "VALUE", default_value_t = 14000000, action, env = "BLADERF_ADSB_BANDWIDTH", help = "Bandwidth")]
  bandwidth: u32,
  
  // Sets the FPGA path
  #[arg(short = None, long = "fpga-path", value_name = "PATH", alias = "fpgapath", action, default_value_t = String::new(), env = "BLADERF_ADSB_FPGA_PATH", help = "FPGA path")]
  fpga_path: String,

  // Sets the target frequency
  #[arg(short = None, long = "frequency", value_name = "hz", action, default_value_t = 1086000000, env = "BLADERF_ADSB_FREQUENCY", help = "Frequency")]
  frequency: u32,

  // Set the gain mode
  #[arg(short = None, long = "gain-mode", value_name = "mode", alias = "lnagain", action, default_value_t = String::from("default"), env = "BLADERF_ADSB_GAIN_MODE", help = "Gain mode", value_parser = ["default", "manual", "fast", "slow", "hybrid"])]
  gain_mode: String,

  // Set the gain level (in dB)
  #[arg(short = None, long = "gain", action, value_name = "db", default_value_t = 35, env = "BLADERF_ADSB_GAIN", help = "Gain in dB")]
  gain: i32,

  // Set the sample rate
  #[arg(short = 'u', long = "sample-rate", alias = "samplerate", action, value_name = "RATE", default_value_t = 16000000, env = "BLADERF_ADSB_SAMPLE_RATE", help = "Sample rate")]
  sample_rate: u32,

  // Enable or disable streaming to remote port
  #[arg(short = None, long = "remote", value_name = "BOOL", action = clap::ArgAction::Set, default_value_t = true, env = "BLADERF_ADSB_REMOTE", help = "Send data to remote server")]
  remote: bool,

  // Set the stream IP address
  #[arg(short = None, long = "remote-ip", action = clap::ArgAction::Set, default_value_t = Ipv4Addr::new(127, 0, 0, 1), env = "BLADERF_ADSB_REMOTE_IP", help = "Remote IP")]
  remote_ip: Ipv4Addr,

  // Set the stream port
  #[arg(short, long = "remote-port", action, default_value_t = 30001, env = "BLADERF_ADSB_REMOTE_PORT", help = "Remove port (matches readsb \"--net-ri-port\")")]
  remote_port: u16,

  // Set the bias tee
  #[arg(short = None, long = "bias-tee", alias = "biastee", action = clap::ArgAction::SetTrue, default_value_t = false, env = "BLADERF_ADSB_BIAS_TEE", help = "State of bias tee")]
  bias_tee: bool,

  // Manage debugging information
  #[arg(short = 'v', long = "log-level", alias = "loglevel", action = clap::ArgAction::Set, default_value_t = String::from("info"), value_parser = ["off", "error", "warn", "info", "debug", "trace"], env = "BLADERF_ADSB_LOG_LEVEL", help = "Log level")]
  log_level: String,
  #[arg(short = None, long = "log-style", alias = "logstyle", action = clap::ArgAction::Set, default_value_t = String::from("auto"), value_parser = ["auto", "always", "never"], env = "BLADERF_ADSB_LOG_STYLE", help = "Manage color for log messages")]
  log_style: String,
}

fn produce(sender: Sender<String>, cli: Cli, running: Arc<AtomicBool>) {
    let fpga_path: String;

    let mut rx_config: BladeRFModuleConfig = BladeRFModuleConfig {
        frequency: cli.frequency.clone(),
        bandwidth: cli.bandwidth.clone(),
        sample_rate: cli.sample_rate.clone(),
        lna_gain: BLADERF_GAIN_DEFAULT,
        vga1: 10,
        vga2: 6,
    };
    let bias_tee: bool = cli.bias_tee.clone();

    // set lna gain
    match cli.gain_mode.as_str() {
      "default" => {
        info!("set lna gain to BLADERF_GAIN_DEFAULT");
        rx_config.lna_gain = BLADERF_GAIN_DEFAULT;
      },
      "manual" => {
        info!("set lna gain to BLADERF_GAIN_MGC");
        rx_config.lna_gain = BLADERF_GAIN_MGC;
      }
      "fast"   => {
        info!("set lna gain to BLADERF_GAIN_FASTATTACK_AGC");
        rx_config.lna_gain = BLADERF_GAIN_FASTATTACK_AGC;
      }
      "slow"   => {
        info!("set lna gain to BLADERF_GAIN_SLOWATTACK_AGC");
        rx_config.lna_gain = BLADERF_GAIN_SLOWATTACK_AGC;
      }
      "hybrid" => {
        info!("set lna gain to BLADERF_GAIN_HYBRID_AGC");
        rx_config.lna_gain = BLADERF_GAIN_HYBRID_AGC;
      }
      _        => {}
    };

  let mut dev = match bladerf::open(None) {
    Ok(dev) => {
      info!("Successfully loaded BladeRF device");
      dev
    },
    Err(_) => panic!("bladerf::open error"),
  };
  let dev_fpga_size = match dev.get_fpga_size() {
    Ok(size) => {
      info!("FPGA size is {:?}", size);
      size
    },
    Err(_) => panic!("dev.get_fpga_size()")
  };
  if !cli.fpga_path.is_empty() {
    fpga_path = cli.fpga_path;
  } else {
    info!("FPGA path not specified. Falling back to default value.");
    fpga_path = match dev_fpga_size {
      BLADERF_FPGA_UNKNOWN => panic!("Unable to determine FPGA size"),
      BLADERF_FPGA_40KLE   => String::from("/usr/share/Nuand/bladeRF/adsbx40.rbf"),
      BLADERF_FPGA_115KLE  => String::from("/usr/share/Nuand/bladeRF/adsbx115.rbf"),
      BLADERF_FPGA_A4      => String::from("/usr/share/Nuand/bladeRF/adsbxA4.rbf"),
      BLADERF_FPGA_A5      => String::from("/usr/share/Nuand/bladeRF/adsbxA5.rbf"),
      BLADERF_FPGA_A9      => String::from("/usr/share/Nuand/bladeRF/adsbxA9.rbf"),
    }
  }

  info!("Loading FPGA image: {}", fpga_path);
  match dev.load_fpga(fpga_path) {
    Ok(_) => {
      info!("Successfully loaded image");
    },
    Err(_) => panic!("bladerf::load_fpga() error"),
  };

  info!("Closing and opening device for new FPGA image");
  dev.close();
  dev = match bladerf::open(None) {
    Ok(dev) => {
      info!("Successfully re-loaded BladeRF device");
      dev
    },
    Err(_) => panic!("bladerf::open error"),
  };

  debug!("Configure module");
  // Configure RX
  dev.set_bias_tee(BLADERF_MODULE_RX, bias_tee).unwrap();

  dev.configure_module(BLADERF_MODULE_RX, rx_config.clone());

  dev.set_gain_mode(BLADERF_MODULE_RX, rx_config.lna_gain).unwrap();
  match rx_config.lna_gain {
    BLADERF_GAIN_MGC => {
      info!("Setting LNA gain to {}dB", cli.gain);
      dev.set_gain(BLADERF_MODULE_RX, cli.gain).unwrap();
    },
    _ => {}
  }

  // Configure RX sample stream
  dev.sync_config(BLADERF_MODULE_RX, BLADERF_FORMAT_SC16_Q11, 2, 1024, Some(1), 5000).unwrap();

  // Enable RX
  dev.enable_module(BLADERF_MODULE_RX, true).unwrap();

  // based on https://github.com/wiedehopf/readsb/blob/dev/sdr_ubladerf.c
  info!("bladeRF: sampling rate:    {:.1} MHz", dev.get_sample_rate(BLADERF_MODULE_RX).unwrap() as f32 / 1e6);
  info!("bladeRF: frequency:        {:.1} MHz", dev.get_frequency(BLADERF_MODULE_RX).unwrap() as f32 / 1e6);
  info!("bladeRF: gain mode:        {:?}",      dev.get_gain_mode(BLADERF_MODULE_RX).unwrap());
  info!("bladeRF: gain:             {}dB",      dev.get_gain(BLADERF_MODULE_RX).unwrap());
  info!("bladeRF: biastee:          {}",        dev.get_bias_tee(BLADERF_MODULE_RX).unwrap());

  let fw_version = dev.fw_version().unwrap();
  let fpga_version = dev.fpga_version().unwrap();
  let devinfo: Struct_bladerf_devinfo = dev.get_devinfo().unwrap();
  trace!("bladeRF: firmware version: {}.{}.{}", fw_version.major, fw_version.minor, fw_version.patch);
  trace!("bladeRF: fpga version:     {}.{}.{}", fpga_version.major, fpga_version.minor, fpga_version.patch);
  trace!("bladeRF: fpga size:        {:?}", dev.get_fpga_size().unwrap());
  info!("bladeRF: serial number:    {}", unsafe { CStr::from_ptr(devinfo.serial.as_ptr()).to_str().unwrap() });
  trace!("bladeRF: usb bus:          {}", devinfo.usb_bus);
  trace!("bladeRF: usb addr:         {}", devinfo.usb_addr);
  info!("bladeRF: usb speed:        {:?}", dev.device_speed());
  info!("bladeRF: instance:         {}", devinfo.instance);
  info!("bladeRF: manufacturer:     {}", unsafe { CStr::from_ptr(devinfo.manufacturer.as_ptr()).to_str().unwrap() });
  info!("bladeRF: product:          {}", unsafe { CStr::from_ptr(devinfo.product.as_ptr()).to_str().unwrap() });

  let mut k: i32;
  let mut end: i32;
  let mut ascii_buf: String;
  let mut messages: [u8; 4096] = [0; 4096];
  let mut message_count: u64 = 0;

  let pb = ProgressBar::new_spinner();
  pb.enable_steady_tick(Duration::from_millis(120));
  pb.set_style(
    ProgressStyle::with_template("{spinner:40..white} {msg}")
      .unwrap()
      // For more spinners check out the cli-spinners project:
      // https://github.com/sindresorhus/cli-spinners/blob/master/spinners.json
      .tick_strings(&[
      "[-===============================================]",
      "[=-==============================================]",
      "[==-=============================================]",
      "[===-============================================]",
      "[====-===========================================]",
      "[=====-==========================================]",
      "[======-=========================================]",
      "[=======-========================================]",
      "[========-=======================================]",
      "[=========-======================================]",
      "[==========-=====================================]",
      "[===========-====================================]",
      "[============-===================================]",
      "[=============-==================================]",
      "[==============-=================================]",
      "[===============-================================]",
      "[================-===============================]",
      "[=================-==============================]",
      "[==================-=============================]",
      "[===================-============================]",
      "[====================-===========================]",
      "[=====================-==========================]",
      "[======================-=========================]",
      "[=======================-========================]",
      "[========================-=======================]",
      "[=========================-======================]",
      "[==========================-=====================]",
      "[===========================-====================]",
      "[============================-===================]",
      "[=============================-==================]",
      "[==============================-=================]",
      "[===============================-================]",
      "[================================-===============]",
      "[=================================-==============]",
      "[==================================-=============]",
      "[===================================-============]",
      "[====================================-===========]",
      "[=====================================-==========]",
      "[======================================-=========]",
      "[=======================================-========]",
      "[========================================-=======]",
      "[=========================================-======]",
      "[==========================================-=====]",
      "[===========================================-====]",
      "[============================================-===]",
      "[=============================================-==]",
      "[==============================================-=]",
      "[===============================================-]",
      "[==============================================-=]",
      "[=============================================-==]",
      "[============================================-===]",
      "[===========================================-====]",
      "[==========================================-=====]",
      "[=========================================-======]",
      "[========================================-=======]",
      "[=======================================-========]",
      "[======================================-=========]",
      "[=====================================-==========]",
      "[====================================-===========]",
      "[===================================-============]",
      "[==================================-=============]",
      "[=================================-==============]",
      "[================================-===============]",
      "[===============================-================]",
      "[==============================-=================]",
      "[=============================-==================]",
      "[============================-===================]",
      "[===========================-====================]",
      "[==========================-=====================]",
      "[=========================-======================]",
      "[========================-=======================]",
      "[=======================-========================]",
      "[======================-=========================]",
      "[=====================-==========================]",
      "[====================-===========================]",
      "[===================-============================]",
      "[==================-=============================]",
      "[=================-==============================]",
      "[================-===============================]",
      "[===============-================================]",
      "[==============-=================================]",
      "[=============-==================================]",
      "[============-===================================]",
      "[===========-====================================]",
      "[==========-=====================================]",
      "[=========-======================================]",
      "[========-=======================================]",
      "[=======-========================================]",
      "[======-=========================================]",
      "[=====-==========================================]",
      "[====-===========================================]",
      "[==-=============================================]",
      "[=-==============================================]",
      "[-===============================================]",
      ]),
    );

  while running.load(Ordering::SeqCst) {
    dev.sync_rx(&mut messages, 1024, None, 5000).unwrap();

    for i in (0..4096).step_by(16) {
      if (messages[i as usize] & 0x01) == 1 {
        if (messages[i as usize + 2] & 0x80) == 0x80 {
          end = 14;
        } else {
          end = 7;
        }

        ascii_buf = String::from("*");
        k = 0;
        while k < end {
          ascii_buf.push_str(&format!("{:02x}", messages[i as usize + 2 + k as usize]));
          k += 1;
        }
        ascii_buf.push_str(";\n") ;

        trace!("Thread 1");
        debug!("ADS-B message is: {}", ascii_buf);

        // only send to other thread if destined for remote socket
        if cli.remote {
          sender.send(ascii_buf).unwrap();
        }

        // update counter
        message_count = message_count + 1;
        pb.set_message(format!("Processing message {}", message_count.separate_with_commas()));
      }
    }
  }


  pb.finish_with_message("Done");
  info!("Closing bladeRF device");
  // Disable RX, shutting down our underlying RX stream
  dev.enable_module(BLADERF_MODULE_RX, false).unwrap();
  dev.close();
}

// References:
// https://docs.rs/clap/latest/clap/enum.ArgAction.html
fn main() {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();


    let cli = Cli::parse();
 
    // setup logging
    let mut builder = Builder::new();
    builder.filter_level(LevelFilter::from_str(cli.log_level.as_str()).unwrap());
    builder.parse_write_style(cli.log_style.as_str());
    builder.init();

    let remote: bool = cli.remote.clone();

    let addr: SocketAddr;
    let mut stream: Option<TcpStream> = None;
    if remote {
      addr = SocketAddr::new(
        IpAddr::V4(cli.remote_ip.clone()),
        cli.remote_port.clone()
      );
      info!("Connecting to {}", addr);
      stream = match TcpStream::connect(addr) {
        Ok(stream) => Some(stream),
        Err(_) => panic!("Unable to connect to socket"),
      };
    }

    ctrlc::set_handler(move || {
      debug!("received Ctrl+C!");
      r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

  // Read messages and print them out
  let (tx, rx) = channel();
  thread::spawn(move || produce(tx, cli, running));
  loop {
    match rx.recv() {
      Ok(a) => {
        trace!("Thread 2");
        if remote {
          // Send ASCII hex message to socket
          if let Some(ref mut stream) = stream {
            let bytes_written = match stream.write(a.as_bytes()) {
              Ok(b) => b,
              Err(_) => panic!("Error sending buffer to server"),
            };

            debug!("Sent {}/{} bytes (\"{}\") to server", bytes_written, a.len(), a.replace("\n", ""));
            if bytes_written < a.len() {
              warn!("Sent {}/{} bytes to server", bytes_written, a.len());
            }

            // Tell TCP to send the buffered data on the wire
            trace!("flush server stream");
            stream.flush().unwrap();
          }
        }
      },
      Err(_) => break,
    }
  }

  return;
}
