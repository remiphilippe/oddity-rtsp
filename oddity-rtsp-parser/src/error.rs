use std::fmt;
use std::error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
  /// An error occurred decoding the header due to incorrect usage
  /// of text encoding by the sender.
  Encoding,
  /// The first line of the head part is malformed.
  FirstLineMalformed {
    line: String
  },
  /// The specified method is not a valid method.
  MethodUnknown {
    line: String,
    method: String
  },
  /// The header first line does have a method, but it does not have
  /// a target URI, which is the required second part of the first
  /// line of the head.
  UriMissing {
    line: String
  },
  /// The header first line does have a method and target URI, but
  /// it does not have a version, which is the required third part
  /// of the first line of the head.
  VersionMissing {
    line: String
  },
  /// The version specifier is incorrect. It should start with "RTSP/"
  /// followed by a digit, "." and another digit.
  VersionMalformed {
    line: String,
    version: String
  },
  /// Header line is missing the header variable.
  HeaderVariableMissing {
    line: String,
  },
  /// Header does not have value.
  HeaderValueMissing {
    line: String,
    var: String,
  },
  /// The Content-Length header is missing, but it is required.
  ContentLengthMissing,
  /// The Content-Length header is not an integer value, or cannot be
  /// converted to an unsigned integer.
  ContentLengthNotInteger {
    value: String,
  },
  /// This occurs when the caller invokes the state machine with a
  /// state that signals that parsing the head part of the request
  /// was already done before.
  HeadAlreadyDone,
  /// This occurs when the caller invokes the state machine with a
  /// state that signals that parsing the body part of the request
  /// was already done before.
  BodyAlreadyDone,
  /// This occurs when the client provided more bytes than expected,
  /// and appending any more bytes to the body would cause it to
  /// become larger than the provided Content-Length.
  BodyOverflow {
    need: usize,
    got: usize,
  },
  /// Metadata was not parsed for some reason.
  MetadataNotParsed,
  /// This occurs when the caller tries to turn the parser into an
  /// actual request, but the parser was not ready yet.
  NotDone,
}

impl fmt::Display for Error {

  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Error::Encoding =>
        write!(f, "encoding incorrect"),
      Error::FirstLineMalformed { line, } =>
        write!(f, "request line malformed: {}", &line),
      Error::MethodUnknown { line, method, } =>
        write!(f, "method unknown: {} (in request line: {})", &method, &line),
      Error::UriMissing { line, } =>
        write!(f, "uri missing in request line: {}", &line),
      Error::VersionMissing { line, } =>
        write!(f, "version missing in request line: {}", &line),
      Error::VersionMalformed { line, version, } =>
        write!(f, "version malformed: {} (in request line: {})", &version, &line),
      Error::HeaderVariableMissing { line, } =>
        write!(f, "header does not have variable: {}", &line),
      Error::HeaderValueMissing { line, var, } =>
        write!(f, "header does not have value: {} (full line: {})", &var, &line),
      Error::ContentLengthMissing =>
        write!(f, "request does not have Content-Length header"),
      Error::ContentLengthNotInteger { value, } =>
        write!(f, "request has invalid value for Content-Length: {}", &value),
      Error::HeadAlreadyDone =>
        write!(f, "head already done (cycle in state machine)"),
      Error::BodyAlreadyDone =>
        write!(f, "body already done (cycle in state machine)"),
      Error::BodyOverflow { need, got, } =>
        write!(f, "received more data than expected for request body: needed {}, but got {} bytes", need, got),
      Error::MetadataNotParsed =>
        write!(f, "metadata not parsed"),
      Error::NotDone =>
        write!(f, "parser not done yet"),
    }
  }

}

impl error::Error for Error {}