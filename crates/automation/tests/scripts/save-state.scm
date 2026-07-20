;; Runtime save-state capture, restore, discard, and invalidation.
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

(test-suite "Runtime save states"
  (test-case "captures, restores, discards, and invalidates states"
(expect-symbol "save without machine"
  (lambda () (save-state #f))
  'neetan/argument)

(call-with-machine '((model . pc9801vm))
  (lambda (machine)
    (if (not (machine-capability? machine 'save-state))
        (fail "save-state must be supported with a machine"))

    ;; Capture, advance, then restore rewinds both counters without an epoch bump.
    (run-frames! machine 4)
    (let* ((state (save-state machine))
           (saved-tick (machine-tick machine))
           (saved-frame (machine-frame machine))
           (saved-epoch (machine-epoch machine)))
      (if (not (machine-state? state))
          (fail "save-state must return an opaque state object"))
      (run-frames! machine 4)
      (if (<= (machine-frame machine) saved-frame)
          (fail "frame counter should advance before restore"))

      (restore-state! machine state)
      (if (not (= (machine-epoch machine) saved-epoch))
          (fail "restore must not advance the epoch"))
      (if (not (= (machine-tick machine) saved-tick))
          (fail "restore must rewind the tick counter"))
      (if (not (= (machine-frame machine) saved-frame))
          (fail "restore must rewind the frame counter"))

      ;; Discard invalidates the opaque state.
      (discard-state! state)
      (expect-symbol "restore discarded"
        (lambda () (restore-state! machine state))
        'neetan/stale-handle)
      (expect-symbol "discard discarded"
        (lambda () (discard-state! state))
        'neetan/stale-handle))

    ;; Hard reset invalidates outstanding states while the machine stays valid.
    (let ((state (save-state machine)))
      (reset! machine 'hard)
      (expect-symbol "restore after reset"
        (lambda () (restore-state! machine state))
        'neetan/stale-handle))

    ;; Startup restoration also invalidates outstanding states.
    (let ((state (save-state machine)))
      (restore-startup! machine)
      (expect-symbol "restore after startup restoration"
        (lambda () (restore-state! machine state))
        'neetan/stale-handle))

    ;; Scoped states are discarded automatically.
    (let ((scoped-state #f))
      (call-with-values
        (lambda ()
          (call-with-saved-state machine
            (lambda (state)
              (set! scoped-state state)
              (values 'first 'second))))
        (lambda (first second)
          (if (not (and (eq? first 'first) (eq? second 'second)))
              (fail "call-with-saved-state must preserve values"))))
      (expect-symbol "restore scoped state"
        (lambda () (restore-state! machine scoped-state))
        'neetan/stale-handle))

    ;; Explicit invalidation inside the callback is compatible with scoped
    ;; cleanup and does not mask returned values.
    (let ((scoped-state #f))
      (check-equal
        'discarded
        (call-with-saved-state machine
          (lambda (state)
            (set! scoped-state state)
            (discard-state! state)
            'discarded)))
      (expect-symbol "restore explicitly discarded scoped state"
        (lambda () (restore-state! machine scoped-state))
        'neetan/stale-handle))

    (let ((scoped-state #f))
      (check-equal
        'reset
        (call-with-saved-state machine
          (lambda (state)
            (set! scoped-state state)
            (reset! machine 'hard)
            'reset)))
      (expect-symbol "restore reset-invalidated scoped state"
        (lambda () (restore-state! machine scoped-state))
        'neetan/stale-handle))

    ;; Error and continuation unwinding discard scoped states.
    (let ((scoped-state #f))
      (expect-symbol "saved state error"
        (lambda ()
          (call-with-saved-state machine
            (lambda (state)
              (set! scoped-state state)
              (error "expected assertion" 'neetan/assertion))))
        'neetan/assertion)
      (expect-symbol "restore error-unwound state"
        (lambda () (restore-state! machine scoped-state))
        'neetan/stale-handle))

    (let ((scoped-state #f))
      (call/cc
        (lambda (escape)
          (call-with-saved-state machine
            (lambda (state)
              (set! scoped-state state)
              (escape #t)))))
      (expect-symbol "restore continuation-unwound state"
        (lambda () (restore-state! machine scoped-state))
        'neetan/stale-handle))))

))
