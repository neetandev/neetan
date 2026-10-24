;; Machine scopes close on errors, assertion unwinding, continuation escape,
;; and failed transactional construction.
(import (scheme base) (neetan automation 1) (neetan test 1))

(define (expect-symbol name thunk symbol)
  (guard (condition
          (#t
           (if (and (error-object? condition)
                    (memq symbol (error-object-irritants condition)))
               #t
               (fail (string-append name ": wrong error")))))
    (thunk)
    (fail (string-append name ": expected an error"))))

(define (expect-error name thunk)
  (guard (condition (#t #t))
    (thunk)
    (fail (string-append name ": expected an error"))))

(define error-machine #f)
(define assertion-machine #f)
(define escaped-machine #f)
(define resume-machine #f)
(define reentry-count 0)
(define saved-continuation #f)
(define continuation-result #f)

(test-suite "Machine scope unwinding"
  (test-case "closes a machine after an error"
(expect-symbol "machine scope error"
  (lambda ()
    (with-machine (machine '((model . pc9801vm)))
      (set! error-machine machine)
      (error "expected error" 'neetan/test-error)))
  'neetan/test-error)
(expect-symbol "machine after error"
  (lambda () (machine-info error-machine))
  'neetan/stale-handle))

(test-case "closes a machine after an assertion"
(expect-symbol "machine scope assertion"
  (lambda ()
    (call-with-machine '((model . pc9801vm))
      (lambda (machine)
        (set! assertion-machine machine)
        (error "expected assertion" 'neetan/assertion))))
  'neetan/assertion)
(expect-symbol "machine after assertion"
  (lambda () (machine-info assertion-machine))
  'neetan/stale-handle))

(test-case "closes a machine after continuation escape"
(call/cc
  (lambda (escape)
    (call-with-machine '((model . pc9801vm))
      (lambda (machine)
        (set! escaped-machine machine)
        (escape #t)))))
(expect-symbol "machine after continuation escape"
  (lambda () (machine-info escaped-machine))
  'neetan/stale-handle))

;; Re-entering the escaped continuation does not resurrect its machine and its
;; repeated dynamic-wind cleanup must not mask the continuation result.
(test-case "does not resurrect a machine after continuation re-entry"
(set! continuation-result
  (call/cc
    (lambda (leave)
      (call-with-machine '((model . pc9801vm))
        (lambda (machine)
          (set! resume-machine machine)
          (call/cc
            (lambda (resume)
              (set! saved-continuation resume)
              (leave 'escaped)))
          (set! reentry-count (+ reentry-count 1))
          (expect-symbol "machine after continuation re-entry"
            (lambda () (machine-info machine))
            'neetan/stale-handle)
          'reentered)))))
(if (= reentry-count 0)
    (saved-continuation 'resume)
    (begin
      (if (not (= reentry-count 1))
          (fail "continuation must re-enter exactly once"))
      (if (not (eq? continuation-result 'reentered))
          (fail "continuation result was masked by cleanup")))))

;; A factory failure and a missing startup image must each leave no active
;; machine or ready event.
(test-case "keeps construction transactional"
(expect-symbol "transactional factory construction"
  (lambda ()
    (call-with-machine '((model . msx))
      (lambda (machine) machine)))
  'neetan/io)

(expect-error "transactional construction"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm)
        (media . ((floppy 0 "missing-startup.d88"))))
      (lambda (machine) machine))))

(expect-symbol "duplicate specification key"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (model . pc9801vm))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "malformed specification entry"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) broken)
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "non-symbol specification key"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (7 . invalid))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "unknown specification key"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (sound-board . |26k|))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "host setting in a specification"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (backend . legacy))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "media setting in a specification"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (fdd1 . "disk.d88"))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "invalid setting value"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (soundboard . |99|))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "printer path in a specification"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (printer . "printer.txt"))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "presentation setting in a specification"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (window-mode . fullscreen))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "configuration file machine key in a specification"
  (lambda ()
    (call-with-machine
      '((model . pc9801vm) (machine . pc9801vx))
      (lambda (machine) machine)))
  'neetan/argument)

(expect-symbol "target and model disagree"
  (lambda ()
    (call-with-machine
      '((target . msx) (model . pc9801vm))
      (lambda (machine) machine)))
  'neetan/argument))

;; A specification takes the machine settings of the configuration file.
(test-case "accepts the machine settings of the configuration file"
  (with-machine (machine '((target . pc98)
                           (model . pc9801vm)
                           (soundboard . |26k|)
                           (graphicboard . none)
                           (bios . off)
                           (boot-device . auto)))
    (check-true (machine? machine))))

;; A successful scope immediately after every unwind proves the slot is free.
(test-case "allows a successful scope after every unwind"
  (with-machine (machine '((model . pc9801vm)))
    (check-true (machine? machine)))))
