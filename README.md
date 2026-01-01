A (very) simple UI program to control the Sony-WF1000XM5 earbuds on Linux & Web. Only a subset of the features are implemented. 

To use the website you need a browser which implements the [Web Serial API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API). Chromium & Chrome work. 
Simply go to https://usering-around.github.io/sony-wf1000xm5-controller/ to use it.

To use the native app, download the binaries for your platform from the [release page](https://github.com/usering-around/sony-wf1000xm5-controller/releases/tag/v0.1.0) or you can build and run locally via  `cargo run --release` or `cargo run --profile superopt` for extra optimizations.

![screenshot of the UI](/example.png?raw=true)


## This program is not affilated with Sony. Use at your own risk.


### Currently implemented features:
- Active Noise Cancelling configuration
- Equalizer Configuration
- Measuring sound pressure
- Autoconnect on app launch
- Getting Codec
- Getting battery levels
