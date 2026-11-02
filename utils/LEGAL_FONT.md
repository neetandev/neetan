# Legal Status of the Font ROM Re-creation

This document records the legal research behind the re-creation of the NEC PC-98 and Fujitsu FM Towns font ROMs
from `.bit` source files. Japan is the jurisdiction of main interest, since both machines and their typefaces
originate there.

## Summary

Japanese law very likely gives the glyph shapes of the NEC and FM Towns font ROMs no protection. A pixel-exact
re-creation of these glyphs in `.bit` source files is therefore defensible in Japan.

The `.bit` format does not change the legal analysis. A transcription of a ROM dump into a text format is still a
copy of the glyphs. The defense rests only on the fact that the copied material has no protection.

## Copyright in Japan

### The Gona U case

The leading case is the Gona U case (ゴナU事件, Shaken v. Morisawa, 平成10年(受)第332号), decided by the First
Petty Bench of the Supreme Court on 7 September 2000. The court held that a printing typeface is a copyrighted
work only if it meets two conditions together:

1. It has a marked originality compared with earlier printing typefaces (従来の印刷用書体に比して顕著な特徴を
   有するといった独創性).
2. It has aesthetic qualities that make it an object of artistic appreciation in itself (それ自体が美術鑑賞の
   対象となり得る美的特性).

Gona U was a commercially successful typeface and still failed this test. A 16x16 or 24x24 bitmap kanji set is
much more constrained by its grid and by the standard JIS character shapes. It was designed for legibility on a
screen. It cannot plausibly meet the test.

### The ROM as a compilation

The ROM as a whole also has no compilation copyright. The glyphs follow the JIS X 0208 code order and its
standard character repertoire. The selection and the arrangement of the glyphs hold no creative choice.

### Font programs

Font software can be a program work under Japanese copyright law. Morisawa enforced its font programs against
DD Tech, a company that installed pirated copies on the computers it sold. The Osaka District Court ordered an
injunction and damages of 80,555,500 yen on 13 May 2004 (平成15年(ワ)第2552号). The copyright in the font
programs was undisputed between the parties in that case. The appeal settled at the Osaka High Court on
28 September 2004 for 80 million yen.

A font ROM holds raw bitmap data with no executable logic. The `.bit` files hold only glyph data and no code from
NEC or Fujitsu. This line of cases does not apply to them.

## Unfair Competition Prevention Act

- Article 2(1)(iii), dead copy of the form of goods: this provision exists since the 1993 Act. The
  protection ends 3 years after the first sale in Japan (Article 19(1)(vi)(a)). For machines of the 1980s and
  early 1990s this protection expired decades ago.
- Article 2(1)(i) and (ii), confusion with a well-known indication: the courts first held in the Typos case
  (タイポス書体事件, Tokyo District Court 10 March 1980, Tokyo High Court 28 April 1982) that a typeface is an
  intangible and therefore no "goods" under the old Act. The Tokyo High Court later held in the Morisawa typeface
  case (24 December 1993, a provisional injunction under the old Article 1(1)(i)) that a typeface traded on
  floppy disks and other media can be "goods". This provision only matters when a typeface is marketed in a way
  that causes confusion with the original vendor. An emulator that renders the characters of the emulated machine
  does not do that.

## Tort under Civil Code Article 709

In the phototypesetting typeface case (写植機用文字書体事件, Morisawa v. NIC) of 8 March 1989, the Osaka District
Court accepted in principle that tort law can protect a truly creative typeface that another party copies
wholesale. It then dismissed all claims. The court could not find that the defendant had mechanically copied the
typeface. The case involved direct competitors in the typeface market. In the Typos case the Tokyo High Court
also rejected a tort claim for the copy of a typeface. No Japanese court has granted tort protection to a
typeface so far.

The Supreme Court narrowed the tort route further in the North Korean films case (北朝鮮映画事件, 平成21年(受)
第602号) of 8 December 2011. The use of material outside copyright protection is a tort only under special
circumstances, such as harm to a legal interest other than the interest copyright protects. A free emulator that
reproduces discontinued hardware does not compete in any font market. This route is the remaining theoretical
risk, and it is small.

## Other Jurisdictions

The project is distributed worldwide.

- United States: typeface designs are not copyrightable (37 CFR 202.1(e), Eltra Corp. v. Ringer, 579 F.2d
  294, 4th Cir. 1978). US practice still accepts copyright in font software. Bitmap glyph data without code is
  outside copyright. The result is the same as in Japan.
- Germany: the Schriftzeichengesetz of 1981 protects registered typefaces for 10 years, with extensions up to
  a maximum of 25 years. A registered design under the Designgesetz also lasts at most 25 years. An
  exceptionally original typeface can in theory be a work of applied art under the Urheberrechtsgesetz. A
  utilitarian bitmap font from the 1980s does not reach that level, and any registration from that time has
  expired.

## Practical Rules

The analysis covers glyph bitmaps only. Each ROM region that is transcribed must be pure font data.
A ROM region that holds code, tables or a BIOS is outside of this assessment.

The project does not claim a clean-room design. The documentation states that the glyphs
reproduce the original hardware typeface and that Japanese law does not protect typefaces of this kind.

## References

- [Wikipedia: Intellectual property protection of typefaces](https://en.wikipedia.org/wiki/Intellectual_property_protection_of_typefaces)
- [ipforce.jp: Gona U judgment, Supreme Court 7 September 2000](https://ipforce.jp/Hanketsu/jiken/no/10725)
- [ben54.jp: Gona U summary](https://www.ben54.jp/news/3309)
- [JPO / JIII: タイプフェイスの保護 (2009), archived copy](https://web.archive.org/web/2022/https://www.jpo.go.jp/news/kokusai/developing/training/textbook/document/index/protection_of_type_face_2009_jp.pdf)
- [Tamura: case note on the phototypesetting typeface case](https://lex.juris.hokudai.ac.jp/coe/articles/tamura/casenote93a.pdf)
- [IIP: タイプフェイスの保護のあり方に関する調査研究](https://www.iip.or.jp/summary/pdf/detail07j/19_05.pdf)
- [JFPI: note on typeface and font program protection](https://www.jfpi.or.jp/files/user/pdf/data/Chizai_no13.pdf)
- [University of Tsukuba: thesis abstract on the legal protection of printing typefaces](https://klis.tsukuba.ac.jp/current-students/pdf/thesis/2018/s1511525-20190109041359A.pdf)
- [courts.go.jp: Morisawa font program judgment, Osaka District Court 13 May 2004](https://www.courts.go.jp/app/files/hanrei_jp/339/010339_hanrei.pdf)
- [ScanNetSecurity: Morisawa font program settlement (2004)](https://scan.netsecurity.ne.jp/article/2004/09/30/13836.html)
- [courts.go.jp: North Korean films case, Supreme Court 8 December 2011](https://www.courts.go.jp/hanrei/81813/detail2/index.html)
- [e-Gov: Unfair Competition Prevention Act](https://laws.e-gov.go.jp/law/405AC0000000047)
- [Weblio: Jiskan](https://www.weblio.jp/content/Jiskan)
