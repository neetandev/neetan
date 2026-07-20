;; Script-driven PC-98 HLE construction and portable boot input.
(import (scheme base) (neetan automation 1) (neetan test 1))

(test-suite "PC-98 construction"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "constructs and accepts portable input"
    (let ((info (machine-info machine)))
      (if (not (eq? (alist-ref info 'target) 'pc98))
          (fail "target should be pc98"))
      (if (not (eq? (alist-ref info 'model) 'pc9801vm))
          (fail "model should be pc9801vm")))

    (if (not (machine-capability? machine 'keyboard))
        (fail "pc98 should report keyboard support"))

    (run-frames! machine 5)
    (if (< (machine-frame machine) 5)
        (fail "machine-frame should advance"))

    (key-tap! machine 'return)
    (type-text! machine "AB")

    (if (<= (machine-emulated-time-ns machine) 0)
        (fail "emulated time should advance")))))
