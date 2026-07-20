;; PC-98 boot-to-title screen check through the test library.
;;
;; Boots the ROM-free PC-98 HLE machine, runs to a stable presentation, and
;; compares the framebuffer against a committed baseline with check-screen. The
;; baseline was generated with save-screenshot! during authoring.
(import
  (scheme base)
  (neetan automation 1)
  (neetan test 1)
  (r7rs receive))

(test-suite "PC-98 title"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "pc98 boots to a presented title screen"
      (run-frames! machine 30)
      (check-true (screen-available? machine))
      (receive (width height) (screen-size machine)
        (check-equal 640 width)
        (check-equal 400 height))
      (check-true (string? (screen-hash machine)))
      (check-screen machine "expected/pc98-title.png")
      (save-screenshot! machine "pc98-title-actual.png"))))
