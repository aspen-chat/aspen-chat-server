#!/usr/bin/env python3
"""Archives the iOS app signed for the App Store, exports it, and with `--upload` sends it to
App Store Connect, where it becomes a TestFlight build. CI runs it for each push to `main`.

    client/packages/mobile/ios/scripts/app_store.py --profile app.mobileprovision \\
        --profile notifications.mobileprovision --build-number 123.1 [--certificate dist.p12] \\
        [--relay https://relay.example] [--upload --api-key AuthKey_X.p8 --api-key-id X \\
        --api-issuer UUID] [--out DIR]

Run it after the web app is built and copied into the project (`pnpm build`, `npx cap sync ios`
in `packages/mobile`). It signs by hand, never with Xcode's automatic signing, which in CI would
make a new development certificate on every run: the Apple Distribution identity comes from
`--certificate`, a `.p12` whose password is `ASPEN_CERTIFICATE_PASSWORD`, imported into a
keychain of its own that is deleted afterwards, or without it from the keychains already
searched. Each `--profile` is an App Store provisioning profile, matched to the target it signs
by the bundle id it names; there must be one for the app and one for its notification service
extension, and each must hold the identity's certificate.

`--build-number` is the bundle version both bundles carry, which must grow with each upload of
one marketing version. `--relay` is the push relay the build names (`AspenPushRelay`, see
`spec/push.md`); without it the build has no push. The archive and the exported `.ipa` are
written under `--out`, `target/ios-app-store/` by default.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import os
import plistlib
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent
REPO = IOS.parents[3]
TEAM = "L2GS25U79G"
BUNDLES = {"com.aspen-chat.client", "com.aspen-chat.client.notifications"}
"""The app and its notification service extension, the bundles an archive of `App` signs."""
PROFILE_DIRECTORIES = [
    Path.home() / "Library" / "Developer" / "Xcode" / "UserData" / "Provisioning Profiles",
    Path.home() / "Library" / "MobileDevice" / "Provisioning Profiles",
]
"""Where Xcode looks for provisioning profiles: the first since Xcode 16, the second before."""


class Failed(Exception):
    pass


def run(*args: str, echo: bool = True, **kwargs) -> subprocess.CompletedProcess:
    """Runs a command, printing it first unless `echo` is false, as it is for one holding a
    password."""
    if echo:
        print(f"$ {' '.join(args)}", flush=True)
    result = subprocess.run(args, **kwargs)
    if result.returncode:
        raise Failed(f"{args[0]} {args[1] if len(args) > 1 else ''} exited with {result.returncode}")
    return result


def output(*args: str) -> bytes:
    return subprocess.run(args, check=True, capture_output=True).stdout


class Profile:
    def __init__(self, path: Path):
        try:
            content = plistlib.loads(output("security", "cms", "-D", "-i", str(path)))
        except (subprocess.CalledProcessError, plistlib.InvalidFileException) as error:
            raise Failed(f"{path} is not a provisioning profile") from error
        self.path = path
        self.uuid: str = content["UUID"]
        self.name: str = content["Name"]
        team, _, self.bundle = content["Entitlements"]["application-identifier"].partition(".")
        if team != TEAM:
            raise Failed(f"{self.name} is team {team}'s; the app is team {TEAM}'s")
        if self.bundle not in BUNDLES:
            raise Failed(f"{self.name} is for {self.bundle}, which the app does not sign")
        # An App Store profile names no devices, which development and ad hoc profiles do, and
        # does not provision every device, which an enterprise one does.
        if "ProvisionedDevices" in content or content.get("ProvisionsAllDevices"):
            raise Failed(f"{self.name} is not an App Store profile; make one of the App Store "
                         f"Connect kind for {self.bundle}")
        # plistlib reads dates as naive UTC.
        if content["ExpirationDate"] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
            raise Failed(f"{self.name} expired on {content['ExpirationDate']:%Y-%m-%d}; make it "
                         "again in the Apple Developer site and replace its secret")
        self.certificates = {hashlib.sha1(der).hexdigest().upper() for der in content["DeveloperCertificates"]}

    def install(self) -> None:
        for directory in PROFILE_DIRECTORIES:
            directory.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(self.path, directory / f"{self.uuid}.mobileprovision")


class Keychain:
    """A keychain of its own for the certificate, put first in the search list while it is open,
    so codesign finds the identity without the login keychain, which CI has none of."""

    def __init__(self, certificate: Path, password: str, work: Path):
        self.path = work / "signing.keychain-db"
        self.password = secrets.token_urlsafe(24)
        self.searched = [line.strip().strip('"') for line in
                         output("security", "list-keychains", "-d", "user").decode().splitlines()]
        run("security", "create-keychain", "-p", self.password, str(self.path), echo=False)
        try:
            # Locked again after six hours, longer than any archive takes, and never on sleep.
            run("security", "set-keychain-settings", "-t", "21600", str(self.path))
            run("security", "unlock-keychain", "-p", self.password, str(self.path), echo=False)
            run("security", "import", str(certificate), "-k", str(self.path), "-f", "pkcs12",
                "-P", password, "-T", "/usr/bin/codesign", "-T", "/usr/bin/security", echo=False)
            # Lets codesign use the key without the dialog asking for the keychain's password.
            run("security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s",
                "-k", self.password, str(self.path), echo=False, stdout=subprocess.DEVNULL)
            run("security", "list-keychains", "-d", "user", "-s", str(self.path), *self.searched)
        except Failed:
            self.close()
            raise

    def close(self) -> None:
        subprocess.run(["security", "list-keychains", "-d", "user", "-s", *self.searched])
        subprocess.run(["security", "delete-keychain", str(self.path)])


def distribution_identity(keychain: Keychain | None) -> tuple[str, str]:
    """The Apple Distribution identity to sign with: its certificate's SHA-1, which profiles
    list their certificates by, and its name."""
    args = ["security", "find-identity", "-v", "-p", "codesigning"]
    if keychain:
        args.append(str(keychain.path))
    found = re.findall(r'\) ([0-9A-F]{40}) "(Apple Distribution: [^"]*\(' + TEAM + r'\))"',
                       output(*args).decode())
    if not found:
        # find-identity -v leaves out an identity whose certificate macOS cannot trust up to
        # Apple's root, which no build signed with it would pass.
        raise Failed(f"no valid Apple Distribution identity of team {TEAM}"
                     + (" in --certificate" if keychain else " in the keychains searched; give --certificate"))
    if len(found) > 1:
        raise Failed("more than one Apple Distribution identity: "
                     + ", ".join(f"{name} ({sha1})" for sha1, name in found))
    return found[0]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--profile", type=Path, action="append", required=True)
    parser.add_argument("--build-number", required=True)
    parser.add_argument("--certificate", type=Path)
    parser.add_argument("--relay", default="")
    parser.add_argument("--upload", action="store_true")
    parser.add_argument("--api-key", type=Path)
    parser.add_argument("--api-key-id")
    parser.add_argument("--api-issuer")
    parser.add_argument("--out", type=Path, default=REPO / "target" / "ios-app-store")
    args = parser.parse_args()

    if args.upload and not (args.api_key and args.api_key_id and args.api_issuer):
        raise Failed("--upload needs --api-key, --api-key-id, and --api-issuer")
    if not re.fullmatch(r"\d+(\.\d+){0,2}", args.build_number):
        raise Failed("--build-number is up to three numbers separated by dots")
    if not (IOS / "App" / "App" / "public" / "index.html").exists():
        raise Failed("the web app is not in the project; run pnpm build and npx cap sync ios "
                     "in packages/mobile first")

    profiles = {}
    for path in args.profile:
        profile = Profile(path)
        if profile.bundle in profiles:
            raise Failed(f"two profiles for {profile.bundle}: {profiles[profile.bundle].name} and {profile.name}")
        profiles[profile.bundle] = profile
    missing = sorted(BUNDLES - profiles.keys())
    if missing:
        raise Failed(f"no profile for {', '.join(missing)}")

    if args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True)
    with tempfile.TemporaryDirectory() as work_name:
        work = Path(work_name)
        keychain = None
        if args.certificate:
            keychain = Keychain(args.certificate, os.environ.get("ASPEN_CERTIFICATE_PASSWORD", ""), work)
        try:
            sha1, identity = distribution_identity(keychain)
            for profile in profiles.values():
                if sha1 not in profile.certificates:
                    raise Failed(f"{profile.name} does not hold the certificate of {identity}; "
                                 "make the profile again with that certificate")
                profile.install()
            archive(args, work, keychain, identity, profiles)
            export(args, work, sha1, profiles)
        finally:
            if keychain:
                keychain.close()


def archive(args, work: Path, keychain: Keychain | None, identity: str, profiles: dict[str, Profile]) -> None:
    # Each target takes the profile of its own bundle id, `com.aspen-chat.client` reading
    # `ASPEN_PROFILE_com_aspen_chat_client`; a target with none, a Swift package's, signs with no
    # profile, as it must. A command-line setting would give every target the same one.
    settings = work / "signing.xcconfig"
    settings.write_text(
        "CODE_SIGN_STYLE = Manual\n"
        f"CODE_SIGN_IDENTITY = {identity}\n"
        f"DEVELOPMENT_TEAM = {TEAM}\n"
        "PROVISIONING_PROFILE_SPECIFIER = $(ASPEN_PROFILE_$(PRODUCT_BUNDLE_IDENTIFIER:c99extidentifier))\n"
        + "".join(f"ASPEN_PROFILE_{re.sub(r'[^A-Za-z0-9]', '_', bundle)} = {profile.uuid}\n"
                  for bundle, profile in profiles.items()))
    run("xcodebuild", "-project", "App.xcodeproj", "-scheme", "App", "-configuration", "Release",
        "-destination", "generic/platform=iOS", "-archivePath", str(args.out / "App.xcarchive"),
        "-xcconfig", str(settings),
        f"CURRENT_PROJECT_VERSION={args.build_number}", f"ASPEN_PUSH_RELAY={args.relay}",
        *([f"OTHER_CODE_SIGN_FLAGS=--keychain {keychain.path}"] if keychain else []),
        "archive", cwd=IOS / "App")


def export(args, work: Path, sha1: str, profiles: dict[str, Profile]) -> None:
    options = work / "ExportOptions.plist"
    options.write_bytes(plistlib.dumps({
        "method": "app-store-connect",
        "destination": "upload" if args.upload else "export",
        "teamID": TEAM,
        "signingStyle": "manual",
        "signingCertificate": sha1,
        "provisioningProfiles": {bundle: profile.uuid for bundle, profile in profiles.items()},
        "manageAppVersionAndBuildNumber": False,
        "uploadSymbols": True,
    }))
    authentication = [
        "-authenticationKeyPath", str(args.api_key.resolve()),
        "-authenticationKeyID", args.api_key_id,
        "-authenticationKeyIssuerID", args.api_issuer,
    ] if args.upload else []
    run("xcodebuild", "-exportArchive", "-archivePath", str(args.out / "App.xcarchive"),
        "-exportPath", str(args.out), "-exportOptionsPlist", str(options), *authentication)


if __name__ == "__main__":
    try:
        main()
    except Failed as failure:
        print(f"app_store.py: {failure}", file=sys.stderr)
        sys.exit(1)
