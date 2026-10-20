export function trustedIpc(frame, rendererUrl) {
  return frame?.mainFrame === true && frame.url === rendererUrl;
}
export function approvalOrigin(address) {
  const u = new URL(address);
  if (
    u.protocol !== "http:" ||
    u.hostname !== "127.0.0.1" ||
    !u.port ||
    u.username ||
    u.password ||
    u.pathname !== "/" ||
    u.search ||
    u.hash
  )
    throw Error("invalid_approval_origin");
  return u.origin;
}
export function allowApprovalNavigation(address, origin) {
  try {
    const u = new URL(address);
    return (
      u.origin === origin &&
      u.pathname === "/" &&
      !u.search &&
      !u.hash &&
      !u.username &&
      !u.password
    );
  } catch {
    return false;
  }
}
export function macBinaryArch(bytes) {
  if (bytes.length < 8 || bytes.readUInt32LE(0) !== 0xfeedfacf)
    throw Error("expected_macho_binary");
  const cpu = bytes.readUInt32LE(4);
  if (cpu === 0x0100000c) return "arm64";
  if (cpu === 0x01000007) return "x64";
  throw Error("unsupported_binary_arch");
}
