;; Hard reset advances the epoch and resets epoch counters.
(import (scheme base) (neetan automation 1) (neetan test 1))

(test-suite "Reset and startup restoration"
  (with-machine (machine '((model . pc9801vm)))
    (test-case "advances epochs and resets counters"
    (run-frames! machine 4)

    (let ((frames-before (machine-frame machine)))
      (reset! machine 'hard)
      (if (not (= (machine-epoch machine) 1))
          (fail "hard reset should advance the epoch"))
      (if (not (= (machine-epoch-tick machine) 0))
          (fail "hard reset should reset epoch ticks"))
      (if (< (machine-frame machine) frames-before)
          (fail "machine frames should stay monotonic across reset")))

    (restore-startup! machine)
    (if (not (= (machine-epoch machine) 2))
        (fail "restore should reconstruct and advance the epoch")))))
