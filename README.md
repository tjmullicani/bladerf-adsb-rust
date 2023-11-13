## Installation

```
$ git clone https://github.com/tjmullicani/bladerf-adsb-rust
$ cd bladerf-adsb-rust/
$ wget https://www.nuand.com/fpga/adsbxA4.rbf
$ make
$ sudo make install
$ bladeRF_adsb --help
```

This will compile and run the user-mode utility that interfaces with the VHDL decoder. The user-mode program loads the prebuilt ADS-B decoder FPGA image. As soon as a message is received from the FPGA it is displayed to the command line and also transmitted to dump1090 for visualization. Once messages get displayed in the command line, they will appear on the local dump1090 HTTP server.
