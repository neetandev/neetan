;; The public (neetan trace 1) library.
(define-library (neetan trace 1)
  (export
    trace-schema trace-start! trace-active? trace-stop! trace-drain!
    trace-failure wait-for-event)
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

    (define default-maximum-ticks-per-frame 50000000)
    (define default-wait-for-event-frames 120)

    (define (wait-for-event machine filter . optional-options)
      (%require-filter "wait-for-event" filter)
      (let* ((options
               (cond
                 ((null? optional-options) '())
                 ((null? (cdr optional-options))
                  (%validate-options
                    "wait-for-event" (car optional-options) '(frames ticks)))
                 (else
                   (error "wait-for-event: expected two or three arguments"
                          'neetan/argument))))
             (missing (cons #f #f))
             (frames-value (alist-ref options 'frames missing))
             (ticks-value (alist-ref options 'ticks missing))
             (maximum-frames
               (if (eq? frames-value missing)
                   default-wait-for-event-frames
                   (%require-count "wait-for-event" frames-value)))
             (maximum-ticks
               (if (eq? ticks-value missing)
                   (* maximum-frames default-maximum-ticks-per-frame)
                   (%require-count "wait-for-event" ticks-value))))
        (%raise-if-error
          (%wait-for-event (%require-machine-token "wait-for-event" machine)
                           filter maximum-frames maximum-ticks))))))
