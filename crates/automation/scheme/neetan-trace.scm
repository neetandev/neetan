;; The public (neetan trace 1) library.
(define-library (neetan trace 1)
  (export
    trace-schema trace-start! trace-active? trace-stop! trace-drain!
    trace-failure wait-for-event save-trace! trace-arm!)
  (import (scheme base) (neetan automation 1) (neetan internal 1)
          (neetan handles internal 1))
  (begin
    (define (%raise-if-error value)
      (if (and (pair? value) (eq? (car value) '%error))
          (error (car (cddr value)) (cadr value))
          value))

    (define (%require-filter who value)
      (if (list? value)
          value
          (error (string-append who ": filter must be an association list or ()")
                 'neetan/argument)))

    (define (%require-count who value)
      (if (and (integer? value) (exact? value) (>= value 0))
          value
          (error (string-append who ": expected a non-negative exact integer")
                 'neetan/argument)))

    (define (%validate-options who options allowed-keys)
      (if (not (list? options))
          (error (string-append who ": options must be an association list")
                 'neetan/argument)
          (let loop ((entries options) (keys '()))
            (if (null? entries)
                options
                (let ((entry (car entries)))
                  (if (or (not (pair? entry)) (not (symbol? (car entry))))
                      (error (string-append who ": malformed option")
                             'neetan/argument)
                      (if (not (memq (car entry) allowed-keys))
                          (error (string-append who ": unknown option")
                                 'neetan/argument)
                          (if (memq (car entry) keys)
                              (error (string-append who ": duplicate option")
                                     'neetan/argument)
                              (loop (cdr entries)
                                    (cons (car entry) keys))))))))))

    (define (trace-schema machine)
      (%raise-if-error
        (%trace-schema (%require-machine-token "trace-schema" machine))))

    (define (trace-start! machine filter)
      (%require-filter "trace-start!" filter)
      (%raise-if-error
        (%trace-start (%require-machine-token "trace-start!" machine) filter))
      (if #f #f))

    (define (trace-active? machine)
      (%trace-active? (%require-machine-token "trace-active?" machine)))

    (define (trace-stop! machine)
      (%raise-if-error
        (%trace-stop (%require-machine-token "trace-stop!" machine)))
      (if #f #f))

    (define (trace-drain! machine)
      (%raise-if-error
        (%trace-drain (%require-machine-token "trace-drain!" machine))))

    (define (trace-failure machine)
      (%raise-if-error
        (%trace-failure (%require-machine-token "trace-failure" machine))))

    (define (%require-string who value)
      (if (string? value)
          value
          (error (string-append who ": expected a string") 'neetan/argument)))

    ;; Writes the buffered trace events to an artifact as Scheme data, one event
    ;; datum per line. The text reads back with `read`, and the events are the
    ;; same alists trace-drain! returns. This is non-consuming, so the buffer
    ;; stays available for a later trace-drain!.
    (define (save-trace! machine path)
      (%require-string "save-trace!" path)
      (%raise-if-error
        (%save-trace (%require-machine-token "save-trace!" machine) path)))

    (define default-maximum-ticks-per-frame 50000000)
    (define default-wait-for-event-frames 120)

    (define (%every predicate items)
      (or (null? items)
          (and (predicate (car items)) (%every predicate (cdr items)))))

    ;; Validates the snapshot option value is a list holding one processor
    ;; symbol.
    ;;
    ;; A snapshot is captured at HLE dispatch entry, so only main-CPU HLE events
    ;; carry register state. Other event classes marshal the snapshot as #f.
    (define (%require-snapshot who value)
      (if (and (list? value) (%every symbol? value))
          value
          (error (string-append who ": snapshot must be a list of processor symbols")
                 'neetan/argument)))

    ;; Runs a triggered bounded ring capture.
    ;;
    ;; The spec is an alist with required keys capture, trigger, before, after,
    ;; and artifact, plus optional frames and ticks run bounds. Events matching
    ;; the capture filter are kept in a window of at most `before` events ahead
    ;; of the first trigger match and `after` events following it. The machine
    ;; stops as soon as the post-trigger context is complete. Once the trigger
    ;; has fired the retained events are written to the artifact in the same
    ;; format save-trace! uses. Returns an alist with triggered, complete,
    ;; events, trigger-index, and bytes entries.
    (define (trace-arm! machine spec)
      (let* ((options (%validate-options "trace-arm!" spec
                        '(capture trigger before after artifact frames ticks)))
             (missing (cons #f #f))
             (%required
               (lambda (key)
                 (let ((value (alist-ref options key missing)))
                   (if (eq? value missing)
                       (error (string-append "trace-arm!: missing required option: "
                                             (symbol->string key))
                              'neetan/argument)
                       value))))
             (capture (%require-filter "trace-arm!" (%required 'capture)))
             (trigger (%require-filter "trace-arm!" (%required 'trigger)))
             (before (%require-count "trace-arm!" (%required 'before)))
             (after (%require-count "trace-arm!" (%required 'after)))
             (artifact (%require-string "trace-arm!" (%required 'artifact)))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (maximum-frames
               (if (eq? frames-value missing)
                   default-wait-for-event-frames
                   (%require-count "trace-arm!" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* maximum-frames default-maximum-ticks-per-frame)
                   (%require-count "trace-arm!" ticks-value))))
        (%raise-if-error
          (%trace-arm (%require-machine-token "trace-arm!" machine)
                      capture trigger before after artifact
                      maximum-frames maximum-ticks))))

    (define (wait-for-event machine filter . optional-options)
      (%require-filter "wait-for-event" filter)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "wait-for-event" (car optional-options) '(frames ticks snapshot)))
                 (else
                   (error "wait-for-event: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (snapshot-value (alist-ref options 'snapshot missing))
             (maximum-frames
               (if (eq? frames-value missing)
                   default-wait-for-event-frames
                   (%require-count "wait-for-event" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* maximum-frames default-maximum-ticks-per-frame)
                   (%require-count "wait-for-event" ticks-value)))
             (snapshot
               (if (eq? snapshot-value missing)
                   '()
                   (%require-snapshot "wait-for-event" snapshot-value))))
        (%raise-if-error
          (%wait-for-event (%require-machine-token "wait-for-event" machine)
                           filter maximum-frames maximum-ticks snapshot))))))
