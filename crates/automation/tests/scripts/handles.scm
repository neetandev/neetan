;; Opaque machine ownership, scoped cleanup, and alist access helpers.
(import (scheme base) (scheme write)
        (neetan automation 1) (neetan test 1))

(define (expect-symbol name thunk symbol)
  (guard (condition
          (#t
           (if (and (error-object? condition)
                    (memq symbol (error-object-irritants condition)))
               #t
               (fail (string-append name ": wrong error")))))
    (thunk)
    (fail (string-append name ": expected an error"))))

(define stale-machine #f)
(define stale-state #f)
(define second-machine #f)

(define (written value)
  (let ((port (open-output-string)))
    (write value port)
    (get-output-string port)))

(test-suite "Opaque handles and alists"
  (test-case "enforces handle ownership and alist contracts"
(check-false (machine? 1))
(check-false (machine? 'machine))
(check-false (machine-state? 1))
(check-false (machine-state? 'state))

(call-with-machine '((model . pc9801vm))
  (lambda (machine)
    (set! stale-machine machine)
    (set! stale-state (save-state machine))
    (check-true (machine? machine))
    (check-true (machine-state? stale-state))
    (check-equal "#<object>" (written machine))
    (check-equal "#<object>" (written stale-state))
    (let ((other-state (save-state machine)))
      (check-true (not (eq? stale-state other-state)))
      (discard-state! other-state))
    (let* ((snapshot (machine-info machine))
           (target (assq 'target snapshot)))
      (set-cdr! target 'corrupted)
      (check-equal 'pc98 (alist-ref (machine-info machine) 'target)))
    (let* ((snapshot (machine-capabilities machine))
           (entry (car snapshot))
           (key (car entry))
           (original (cdr entry)))
      (set-cdr! entry 'corrupted)
      (check-equal original
                   (alist-ref (machine-capabilities machine) key)))
    (expect-symbol "restore integer state"
      (lambda () (restore-state! machine 1))
      'neetan/argument)
    (expect-symbol "restore symbol state"
      (lambda () (restore-state! machine 'state))
      'neetan/argument)
    (expect-symbol "discard integer state"
      (lambda () (discard-state! 1))
      'neetan/argument)
    (expect-symbol "discard symbol state"
      (lambda () (discard-state! 'state))
      'neetan/argument)
    (expect-symbol "nested machine"
      (lambda ()
        (call-with-machine '((model . pc9801vm))
          (lambda (nested) nested)))
      'neetan/machine-state)))

(expect-symbol "stale machine"
  (lambda () (machine-info stale-machine))
  'neetan/stale-handle)

(call-with-machine '((model . pc9801vm))
  (lambda (machine)
    (set! second-machine machine)
    (check-true (not (eq? stale-machine machine)))
    (expect-symbol "cross-machine state"
      (lambda () (restore-state! machine stale-state))
      'neetan/stale-handle)))

(check-true (not (eq? stale-machine second-machine)))

(call-with-values
  (lambda ()
    (with-machine (machine '((model . pc9801vm)))
      (values (machine? machine) 42)))
  (lambda (is-machine answer)
    (check-true is-machine)
    (check-equal 42 answer)))

(let ((descriptor '((target . pc98) (model . pc9801vm))))
  (check-equal 'pc98 (alist-ref descriptor 'target))
  (check-equal 'fallback (alist-ref descriptor 'missing 'fallback))
  (check-true (alist-key? descriptor 'model))
  (check-false (alist-key? descriptor 'missing)))

(expect-symbol "duplicate alist key"
  (lambda () (alist-ref '((key . 1) (key . 2)) 'key))
  'neetan/argument)

(expect-symbol "unrelated duplicate alist key"
  (lambda () (alist-ref '((key . 1) (other . 2) (other . 3)) 'key))
  'neetan/argument)

(expect-symbol "missing alist key"
  (lambda () (alist-ref '((key . 1)) 'missing))
  'neetan/argument)

(expect-symbol "malformed alist entry"
  (lambda () (alist-ref '((key . 1) broken) 'key))
  'neetan/argument)

(expect-symbol "non-symbol alist key"
  (lambda () (alist-key? '((1 . value)) 'key))
  'neetan/argument)

(expect-symbol "improper alist"
  (lambda () (alist-ref '((key . 1) . tail) 'key))
  'neetan/argument)

))
