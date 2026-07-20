;; PC-98 tracing through the public (neetan trace 1) library.
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

(test-suite "PC-98 tracing"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))

(test-case "trace capability and schema discovery"
  (check-true (machine-capability? machine 'trace))
  (let ((schema (trace-schema machine)))
      (check-equal 1 (field 'schema-version schema))
      ;; The four baseline classes are always emitted.
      (check-true (and (memq 'access (field 'supported-classes schema)) #t))
      (check-true (and (memq 'presentation (field 'supported-classes schema)) #t))
      ;; PC-98 emits FDC device events and HLE call providers.
      (let ((devices (map (lambda (entry) (field 'device entry))
                          (field 'devices schema))))
        (check-true (and (memq 'pc98.fdc devices) #t)))
      (let ((providers (map (lambda (entry) (field 'provider entry))
                            (field 'providers schema))))
        (check-true (and (memq 'neetan.dos providers) #t)))
      ;; Queue limits are reported.
      (check-true (> (field 'event-capacity (field 'queue-limits schema)) 0))
      (set-cdr! (assq 'schema-version schema) 99)
    (check-equal 1 (field 'schema-version (trace-schema machine)))))

(test-case "filtered continuous collection and drain"
  (check-false (trace-active? machine))
    (trace-start! machine '((class . scheduled)))
    (check-true (trace-active? machine))
    (run-ticks! machine 400000)
    (let ((events (trace-drain! machine)))
      (check-true (> (length events) 0))
      ;; Every collected event honours the class filter and carries the
      ;; normalized envelope keys.
      (for-each
        (lambda (event)
          (check-equal 'scheduled (field 'class event))
          (check-equal 1 (field 'schema-version event))
          (check-true (exact? (field 'tick event)))
          (check-true (pair? (field 'data event))))
        events))
    (trace-stop! machine)
  (check-false (trace-active? machine)))

(test-case "wait-for-event stops on the first match"
  (let ((event (wait-for-event machine '((class . presentation)))))
      (check-true (pair? event))
      (check-equal 'presentation (field 'class event))
      (check-true (exact? (field 'sequence event))))
  (check-true
    (pair? (wait-for-event machine '((class . presentation)) '())))
    ;; The one-shot collector is stopped again on return.
  (check-false (trace-active? machine)))

(test-case "nested data filters and ranges select an event"
  (let ((event
          (wait-for-event machine
            '((class . presentation)
              (data . ((frame . (range 1 2000))
                       (width . 640))))
            '((frames . 2000)))))
    (check-true (pair? event))
    (check-equal 'presentation (field 'class event))
    (check-equal 640 (field 'width (field 'data event)))))

(test-case "wait-for-event bounds can be overridden independently"
  (check-true
    (pair?
      (wait-for-event machine '((class . presentation))
        '((ticks . 50000000)))))
  (check-true
    (pair?
      (wait-for-event machine '((class . presentation))
        '((frames . 1) (ticks . 50000000))))))

(test-case "wait-for-event is invalid during continuous collection"
  (trace-start! machine '())
    (check-true
      (raises? 'neetan/trace-state
               (lambda ()
                 (wait-for-event machine '() '((frames . 1))))))
  (trace-stop! machine))

(test-case "invalid filters are rejected before execution"
  (check-true (raises? 'neetan/argument
                         (lambda () (trace-start! machine '((bogus . 1))))))
    (check-true (raises? 'neetan/argument
                         (lambda () (trace-start! machine '((class . not-a-class))))))
    (check-true (raises? 'neetan/argument
                         (lambda ()
                         (trace-start! machine
                           '((data . ((address . (range 100 10))))))))))

(test-case "wait-for-event options use the argument contract"
  (check-true
    (raises? 'neetan/argument
      (lambda () (wait-for-event machine '() #f))))
  (check-true
    (raises? 'neetan/argument
      (lambda () (wait-for-event machine '() '((unknown . 1))))))
  (check-true
    (raises? 'neetan/argument
      (lambda ()
        (wait-for-event machine '() '((frames . 1) (frames . 2))))))
  (check-true
    (raises? 'neetan/argument
      (lambda () (wait-for-event machine '() '((ticks . -1))))))
  (check-true
    (raises? 'neetan/argument
      (lambda () (wait-for-event machine '() 120 5000000)))))

(test-case "queue overflow is reported and buffered events are preserved"
  ;; Collecting every class fills the bounded queue quickly.
  (trace-start! machine '())
    (check-true
      (raises? 'neetan/trace-overflow
               (lambda () (run-frames! machine 30 5000000000))))
    (let ((failure (trace-failure machine)))
      (check-true (pair? failure))
      (check-equal 'queue-overflow (field 'reason failure)))
    ;; The earliest complete events remain available for triage.
  (check-true (> (length (trace-drain! machine)) 0))
  (check-false (trace-active? machine)))))
