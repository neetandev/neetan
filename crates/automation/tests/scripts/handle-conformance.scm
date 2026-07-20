;; Every machine-dependent public procedure rejects wrong and stale handles.
(import (scheme base)
        (neetan automation 1)
        (neetan inspect 1)
        (neetan mutate 1)
        (neetan trace 1)
        (neetan test 1))

(define (expect-symbol name thunk symbol)
  (guard (condition
          (#t
           (if (and (error-object? condition)
                    (memq symbol (error-object-irritants condition)))
               #t
               (fail (string-append name ": wrong error")))))
    (thunk)
    (fail (string-append name ": expected an error"))))

(define probes
  (list
    (cons "machine-info" (lambda (machine) (machine-info machine)))
    (cons "machine-capabilities" (lambda (machine) (machine-capabilities machine)))
    (cons "machine-capability?" (lambda (machine) (machine-capability? machine 'keyboard)))
    (cons "machine-epoch" (lambda (machine) (machine-epoch machine)))
    (cons "machine-tick" (lambda (machine) (machine-tick machine)))
    (cons "machine-frame" (lambda (machine) (machine-frame machine)))
    (cons "machine-epoch-tick" (lambda (machine) (machine-epoch-tick machine)))
    (cons "machine-epoch-frame" (lambda (machine) (machine-epoch-frame machine)))
    (cons "machine-emulated-time-ns" (lambda (machine) (machine-emulated-time-ns machine)))
    (cons "machine-shutdown-requested?" (lambda (machine) (machine-shutdown-requested? machine)))
    (cons "run-ticks!" (lambda (machine) (run-ticks! machine 0)))
    (cons "run-frames!" (lambda (machine) (run-frames! machine 0)))
    (cons "run-until-frame!"
          (lambda (machine)
            (run-until-frame! machine 0 '((ticks . 0)))))
    (cons "wait-until"
          (lambda (machine)
            (wait-until machine (lambda () #t)
              '((frames . 0) (ticks . 0)))))
    (cons "reset!" (lambda (machine) (reset! machine 'hard)))
    (cons "restore-startup!" (lambda (machine) (restore-startup! machine)))
    (cons "save-state" (lambda (machine) (save-state machine)))
    (cons "call-with-saved-state"
          (lambda (machine) (call-with-saved-state machine (lambda (state) state))))
    (cons "key-down!" (lambda (machine) (key-down! machine 'a)))
    (cons "key-up!" (lambda (machine) (key-up! machine 'a)))
    (cons "key-tap!"
          (lambda (machine)
            (key-tap! machine 'a '((frames . 0) (ticks . 0)))))
    (cons "type-text!"
          (lambda (machine)
            (type-text! machine "" '((frames . 0) (ticks . 0)))))
    (cons "joystick-set!" (lambda (machine) (joystick-set! machine 0 'up #f)))
    (cons "joystick-clear!" (lambda (machine) (joystick-clear! machine 0)))
    (cons "mouse-move!" (lambda (machine) (mouse-move! machine 0 0)))
    (cons "mouse-button!" (lambda (machine) (mouse-button! machine 'left #f)))
    (cons "media-insert!" (lambda (machine) (media-insert! machine 'floppy 0 "missing.d88")))
    (cons "media-eject!" (lambda (machine) (media-eject! machine 'floppy 0)))
    (cons "media-flush!" (lambda (machine) (media-flush! machine)))
    (cons "media-info" (lambda (machine) (media-info machine 'floppy 0)))
    (cons "screen-available?" (lambda (machine) (screen-available? machine)))
    (cons "screen-size" (lambda (machine) (screen-size machine)))
    (cons "screen-rgba" (lambda (machine) (screen-rgba machine)))
    (cons "screen-pixel" (lambda (machine) (screen-pixel machine 0 0)))
    (cons "screen-hash" (lambda (machine) (screen-hash machine)))
    (cons "save-screenshot!" (lambda (machine) (save-screenshot! machine "screen.png")))
    (cons "screen-matches?"
          (lambda (machine) (screen-matches? machine "missing.png")))
    (cons "screen-region-matches?"
          (lambda (machine)
            (screen-region-matches? machine "missing.png" 0 0 1 1)))
    (cons "wait-for-screen"
          (lambda (machine) (wait-for-screen machine "missing.png" '((frames . 0)))))
    (cons "processors" (lambda (machine) (processors machine)))
    (cons "processor-info" (lambda (machine) (processor-info machine 'cpu.main)))
    (cons "registers" (lambda (machine) (registers machine 'cpu.main)))
    (cons "register-ref" (lambda (machine) (register-ref machine 'cpu.main 'ax)))
    (cons "protected-mode-state"
          (lambda (machine) (protected-mode-state machine 'cpu.main)))
    (cons "address-spaces" (lambda (machine) (address-spaces machine)))
    (cons "address-space-info"
          (lambda (machine) (address-space-info machine 'cpu.main.memory)))
    (cons "memory-read-bytevector"
          (lambda (machine) (memory-read-bytevector machine 'cpu.main.memory 0 1)))
    (cons "memory-peek-unsigned"
          (lambda (machine) (memory-peek-unsigned machine 'cpu.main.memory 0 1 'little)))
    (cons "register-set!"
          (lambda (machine) (register-set! machine 'cpu.main 'ax 0)))
    (cons "memory-write-bytevector!"
          (lambda (machine) (memory-write-bytevector! machine 'cpu.main.memory 0 (bytevector 0))))
    (cons "memory-poke-unsigned!"
          (lambda (machine) (memory-poke-unsigned! machine 'cpu.main.memory 0 1 'little 0)))
    (cons "trace-schema" (lambda (machine) (trace-schema machine)))
    (cons "trace-start!" (lambda (machine) (trace-start! machine '())))
    (cons "trace-active?" (lambda (machine) (trace-active? machine)))
    (cons "trace-stop!" (lambda (machine) (trace-stop! machine)))
    (cons "trace-drain!" (lambda (machine) (trace-drain! machine)))
    (cons "trace-failure" (lambda (machine) (trace-failure machine)))
    (cons "wait-for-event"
          (lambda (machine)
            (wait-for-event machine '() '((frames . 0) (ticks . 0)))))
    (cons "check-screen"
          (lambda (machine) (check-screen machine "missing.png")))))

(define (check-probes value symbol suffix)
  (for-each
    (lambda (probe)
      (expect-symbol (string-append (car probe) suffix)
                     (lambda () ((cdr probe) value))
                     symbol))
    probes))

(define (exercise-valid-probes machine)
  (for-each
    (lambda (probe)
      ;; Capability, media, screen, and mutation domain errors are valid
      ;; outcomes here. The purpose of this pass is to cross every public
      ;; machine boundary with a genuine live record.
      (guard (condition (#t #t))
        ((cdr probe) machine))
      (if (not (pair? (machine-info machine)))
          (fail (string-append (car probe) ": live handle was lost"))))
    probes))

(define stale-machine #f)
(define stale-state #f)

(test-suite "Handle conformance"
  (test-case "rejects wrong and stale handles"
(check-probes 7 'neetan/argument " integer")
(check-probes 'machine 'neetan/argument " symbol")

(call-with-machine '((model . pc9801vm))
  (lambda (machine)
    (set! stale-machine machine)
    (set! stale-state (save-state machine))
    (exercise-valid-probes machine)))

(check-probes stale-machine 'neetan/stale-handle " stale")

(call-with-machine '((model . pc9801vm))
  (lambda (machine)
    (check-probes stale-machine 'neetan/stale-handle " previous scope")
    (expect-symbol "restore cross-machine state"
                   (lambda () (restore-state! machine stale-state))
                   'neetan/stale-handle)
    (expect-symbol "discard closed state"
                   (lambda () (discard-state! stale-state))
                   'neetan/stale-handle)))

))
