;; Hard reset advances the epoch and resets epoch counters.
(import (scheme base) (neetan automation 1) (neetan trace 1) (neetan test 1))

;; Returns the value for key in an alist, or #f.
(define (field key alist)
  (let ((entry (assq key alist)))
    (and entry (cdr entry))))

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

    ;; Trace envelopes carry the advanced epoch after a reset.
    (let ((event (wait-for-event machine '((class . presentation)))))
      (if (not (= (field 'epoch event) 1))
          (fail "trace events should carry the post-reset epoch")))

    (restore-startup! machine)
    (if (not (= (machine-epoch machine) 2))
        (fail "restore should reconstruct and advance the epoch"))

    ;; Trace envelopes carry the advanced epoch after a restore too.
    (let ((event (wait-for-event machine '((class . presentation)))))
      (if (not (= (field 'epoch event) 2))
          (fail "trace events should carry the post-restore epoch"))))))
