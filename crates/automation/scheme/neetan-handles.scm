;; Private opaque resource records shared by the public Neetan libraries.
(define-library (neetan handles internal 1)
  (export
    %make-machine-handle %machine-handle? %require-machine-token
    %make-state-handle %state-handle? %state-owner %state-token)
  (import (scheme base) (neetan internal 1))
  (begin
    (define (%raise-if-error value)
      (if (and (pair? value) (eq? (car value) '%error))
          (error (car (cddr value)) (cadr value))
          value))

    (define-record-type <neetan-machine>
      (%make-machine-handle token)
      %machine-handle?
      (token %machine-handle-token))

    (define-record-type <neetan-machine-state>
      (%make-state-handle owner token)
      %state-handle?
      (owner %state-owner)
      (token %state-token))

    (define (%require-machine-token who machine)
      (if (%machine-handle? machine)
          (let ((token (%machine-handle-token machine)))
            (%raise-if-error (%validate-machine token))
            token)
          (error (string-append who ": expected a machine")
                 'neetan/argument)))))
