;; Explicit errors for unsupported controls and characters.
(import (scheme base) (neetan automation 1) (neetan test 1))

(define (expect-symbol name thunk symbol)
  (call/cc
    (lambda (k)
      (with-exception-handler
        (lambda (condition)
          (let ((irritants
                  (if (error-object? condition)
                      (error-object-irritants condition)
                      '())))
            (if (not (memq symbol irritants))
                (fail (string-append name ": wrong error")))
            (k #f)))
        (lambda ()
          (thunk)
          (fail (string-append name ": expected an error"))
          (k #f))))))

(test-suite "Input errors"
  (with-machine (machine '((model . pc9801vm)))
    (test-case "reports unsupported controls and characters"
    ;; A hiragana character is outside the Version-1 text set.
    (expect-symbol "type-text non-ascii"
      (lambda () (type-text! machine "\x3042;"))
      'neetan/argument)
    ;; Soft reset is not implemented for PC-98.
    (expect-symbol "soft reset"
      (lambda () (reset! machine 'soft))
      'neetan/unsupported)
    ;; PC-98 exposes no joystick port.
    (expect-symbol "joystick port"
      (lambda () (joystick-set! machine 0 'up #t))
      'neetan/unsupported)
    ;; An unknown key name is a bad argument.
    (expect-symbol "unknown key"
      (lambda () (key-down! machine 'no-such-key))
      'neetan/argument))))
