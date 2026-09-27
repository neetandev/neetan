# ymfm_oxide 

Memory safe Rust reimplementation of [ymfm](https://github.com/aaronsgiles/ymfm) for the Yamaha FM synthesis chips.

## Supported chips

| Chip    | Family    | Features                                             |
|---------|-----------|------------------------------------------------------|
| YM2149  | SSG       | 3-ch square wave + noise + envelope, I/O ports       |
| YM2203  | OPN       | 3-ch FM + 3-ch SSG                                   |
| YM2608  | OPNA      | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B             |
| YM2610  | OPNB      | 4-ch stereo FM + SSG + ADPCM-A + ADPCM-B             |
| YM2610B | OPNB2     | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B             |
| YM2612  | OPN2      | 6-ch stereo FM + channel-6 DAC, 9-bit DAC output     |
| YM3438  | OPN2C     | 6-ch stereo FM + channel-6 DAC                       |
| YMF276  | OPN2L     | 6-ch stereo FM + channel-6 DAC, 16-bit output        |
| YMF288  | OPN3L     | 6-ch stereo FM + SSG + ADPCM-A                       |
| YM2151  | OPM       | 8-ch stereo FM + noise + LFO                         |
| YM2164  | OPP       | YM2151 variant                                       |
| YM2414  | OPZ       | 8-ch stereo FM, 8 waveforms, 2 LFOs, fixed frequency |
| YM3806  | OPQ       | 8-ch stereo FM, 2 waveforms, reverb                  |
| YM3533  | OPQ       | YM3806 variant                                       |
| YM3526  | OPL       | 9-ch mono FM                                         |
| Y8950   | MSX-Audio | 9-ch mono FM + ADPCM-B                               |
| YM3812  | OPL2      | 9-ch mono FM, 4 waveforms                            |
| YMF262  | OPL3      | 18-ch 4-output FM, 8 waveforms, 4-op mode            |
| YMF289B | OPL3L     | 18-ch 2-output FM, readable registers                |
| YMF278B | OPL4      | OPL3 FM + 24-ch wavetable PCM                        |
| YM2413  | OPLL      | 9-ch mono FM + rhythm, 15 instruments                |
| YM2423  | OPLL-X    | YM2413 with its own instrument set                   |
| YMF281  | OPLLP     | YM2413 with its own instrument set                   |
| DS1001  | VRC7      | YM2413 with the Konami VRC7 instrument set           |

## License

This project is licensed under [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.
