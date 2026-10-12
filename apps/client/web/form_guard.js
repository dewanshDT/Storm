// Flutter web keeps text fields in hidden <form method="post" action="#">
// elements for autofill, and cancels their `submit` event. A native
// form.submit() — a password manager capturing a sign-in, say — fires no
// event, so the browser POSTed the page and the app reloaded mid sign-in
// (PLAN decision 90). Route it through requestSubmit(), which fires the event
// Flutter cancels: the browser still sees a submission, the page stays.
(function () {
  const submit = HTMLFormElement.prototype.submit;
  HTMLFormElement.prototype.submit = function () {
    if (this.closest('flutter-view') && this.requestSubmit) {
      this.requestSubmit();
      return;
    }
    return submit.call(this);
  };
})();
