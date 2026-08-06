/* eslint-disable no-alert -- this prelude is interactive on purpose */
alert("inside the block");
/* eslint-enable no-alert */
console.log("trailing"); // eslint-disable-line no-console -- the trailing form
// eslint-disable-next-line no-console -- the next-line form
console.log("next line");
/* eslint-disable-next-line no-console
   -- a reason that spans
      several lines */
console.log("multi line");
/* eslint-disable */
console.log("everything from here on");
alert("including this");

// llmlint: ignore-file[suppressions_justified] fixture input, not production code:
// these lines exist to exercise every directive form the parser claims, including
// the blanket `eslint-disable` whose reason-less shape the e2e asserts is reported.
