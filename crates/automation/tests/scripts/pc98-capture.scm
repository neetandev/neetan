;; PC-98 triggered bounded ring capture through (neetan trace 1).
(import (scheme base)
        (neetan automation 1)
        (neetan trace 1)
        (neetan test 1))

;; Returns #t when thunk raises an error carrying the given neetan symbol.
(define (raises? symbol thunk)
  (guard (condition
          (#t (and (error-object? condition)
                   (memq symbol (error-object-irritants condition))
                   #t)))
    (thunk)
    #f))

;; Returns the value for key in an alist, or #f.
(define (field key alist)
  (let ((entry (assq key alist)))
    (and entry (cdr entry))))

(test-suite "PC-98 triggered ring capture"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))

(test-case "trace-arm! validates its arguments before running"
  ;; Missing required keys.
  (check-true (raises? 'neetan/argument
    (lambda () (trace-arm! machine '((capture . ()))))))
  ;; Unknown option key.
  (check-true (raises? 'neetan/argument
    (lambda ()
      (trace-arm! machine
        '((capture . ()) (trigger . ()) (before . 1) (after . 1)
          (artifact . "a.scm") (bogus . 1))))))
  ;; An unknown provider-specific field in the trigger is rejected up front.
  (check-true (raises? 'neetan/argument
    (lambda ()
      (trace-arm! machine
        '((capture . ((class . scheduled)))
          (trigger . ((class . device)
                      (data . ((device . neetan.dos.stdout) (action . write)
                               (fields . ((bogus . 1)))))))
          (before . 4) (after . 4)
          (artifact . "never.scm"))))))
  ;; The artifact path is confined to the artifact root before the machine
  ;; runs.
  (check-true (raises? 'neetan/path-escape
    (lambda ()
      (trace-arm! machine
        '((capture . ((class . scheduled)))
          (trigger . ((class . presentation)))
          (before . 1) (after . 1)
          (artifact . "../escape.scm")))))))

(test-case "triggered capture retains the before and after windows"
  ;; Trigger on a presentation five frames ahead, so several frames of
  ;; scheduled events precede the trigger and saturate the pre-window.
  (let* ((sync (wait-for-event machine '((class . presentation))))
         (target (+ (field 'frame (field 'data sync)) 5))
         (result (trace-arm! machine
                   (list (cons 'capture '((class . scheduled)))
                         (cons 'trigger
                               (list (cons 'class 'presentation)
                                     (cons 'data (list (cons 'frame target)))))
                         (cons 'before 4)
                         (cons 'after 3)
                         (cons 'artifact "capture.scm")))))
    (check-true (field 'triggered result))
    (check-true (field 'complete result))
    ;; The pre-trigger window is full, the trigger event follows it, and
    ;; exactly `after` events complete the capture.
    (check-equal 4 (field 'trigger-index result))
    (check-equal 8 (field 'events result))
    (check-true (> (field 'bytes result) 0)))
  ;; The capture disarms itself.
  (check-false (trace-active? machine)))

(test-case "an unfired trigger keeps storage bounded and writes no artifact"
  (let ((result (trace-arm! machine
                  '((capture . ((class . scheduled)))
                    (trigger . ((class . device)
                                (data . ((device . neetan.dos.vector)
                                         (action . set)))))
                    (before . 2)
                    (after . 2)
                    (frames . 2)
                    (artifact . "untriggered.scm")))))
    (check-false (field 'triggered result))
    (check-false (field 'complete result))
    (check-false (field 'trigger-index result))
    (check-false (field 'bytes result))
    ;; The pre-trigger ring stays bounded no matter how long the run is.
    (check-true (<= (field 'events result) 2)))
  (check-false (trace-active? machine)))

(test-case "trace-arm! is invalid during continuous collection"
  (trace-start! machine '((class . scheduled)))
  (check-true (raises? 'neetan/trace-state
    (lambda ()
      (trace-arm! machine
        '((capture . ((class . scheduled)))
          (trigger . ((class . presentation)))
          (before . 1) (after . 1)
          (artifact . "blocked.scm"))))))
  (trace-stop! machine))))
