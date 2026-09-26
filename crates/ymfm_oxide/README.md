# ymfm_oxide 

Memory safe Rust reimplementation of [ymfm](https://github.com/aaronsgiles/ymfm) for the Yamaha FM synthesis chips.

## Ported chips

Not all chips are ported.

| Chip    | Family | Features                                  |
|---------|--------|-------------------------------------------|
| YM2203  | OPN    | 3-ch FM + 3-ch SSG                        |
| YM2608  | OPNA   | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
| YM2610  | OPNB   | 4-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
| YM2610B | OPNB2  | 6-ch stereo FM + SSG + ADPCM-A + ADPCM-B  |
| YM3526  | OPL    | 9-ch mono FM                              |
| Y8950   | OPL    | 9-ch mono FM + ADPCM-B                    |
| YM3812  | OPL2   | 9-ch mono FM, 4 waveforms                 |
| YMF262  | OPL3   | 18-ch 4-output FM, 8 waveforms, 4-op mode |

## License

This project is licensed under [3-clause BSD](https://opensource.org/license/bsd-3-clause) license.
