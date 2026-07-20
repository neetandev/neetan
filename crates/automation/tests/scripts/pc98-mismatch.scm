;; A deliberately failing check-screen.
;;
;; The baseline is a solid magenta image, so check-screen fails, records the
;; assertion, writes a side-by-side comparison image, and the suite sets one
;; ERROR result. Used to verify the failure and comparison-image behavior.
(import
  (scheme base)
  (neetan automation 1)
  (neetan test 1))

(test-suite "PC-98 mismatch"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "pc98 title does not match the wrong baseline"
      (run-frames! machine 30)
      (check-screen machine "expected/pc98-wrong.png"))))
