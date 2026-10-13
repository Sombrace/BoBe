# Physical BoBe

A physical body is another place to meet the same companion. It contributes a
display, buttons, audio or sensors. The BoBe runtime retains memory, goals,
provider access, tools and conversation authority.

The [ESP32 SDK](../esp32-sdk/README.md) provides text-terminal, sensor and voice-body
examples. The WonderBoy WB-12 is the reference voice body. It is prototype hardware,
not a production device sold with BoBe.

## Connection and ownership

BodyLink uses a dedicated mTLS connection. Each enrolled body has its own identity
and explicitly granted capabilities. Multiple bodies can remain connected without
sharing their credentials or becoming separate companions.

Conversation and media output are targeted to the owning endpoint and connection.
Bounded playback credit and acknowledgements report progress. A connection or
runtime restart must not replay uncertain actions or send one body's output to
another.

Physical-body enrollment is separate from Mac and iPhone pairing. Bodies use
provisioned certificates; they do not enter the phone's six-digit code or receive
its credential. Follow the [SDK enrollment guide](../esp32-sdk/README.md#enroll-and-connect).

## Available surfaces

| Surface | Behavior |
| --- | --- |
| Text terminal | Send text or an action and display the response on its own conversation channel |
| Sensor | Submit bounded observations within granted capabilities |
| Voice body | Capture push-to-talk audio, show state and play the runtime's targeted response |

The runtime owns model calls, turn ordering and persistence. The device owns its
hardware and real-time input/output. A device's reported capabilities cannot expand
the owner's grants.

## Voice and privacy

Push-to-talk is the supported WB-12 interaction. The reference body is half-duplex;
production wake-word and full-duplex acoustic performance are not qualified.
One body at a time holds the active voice turn.

Physical mute and visible listening state must reflect actual capture. A body that
cannot reach its runtime must report that state rather than imply it is answering
or retaining audio for later. Provider and administrative credentials stay with
the runtime.

The SDK supports signed A/B firmware updates and application rollback. Factory
firmware and device-specific recovery images are not distributed with the SDK.

## Limits

The product does not provide a camera, autonomous movement or a production
consumer-device enrollment system.

Source checks and protocol simulations do not establish physical qualification.
Use the [SDK validation guide](../esp32-sdk/docs/validation.md) and the actual
hardware before making claims about acoustics, battery accuracy, thermal behavior,
updates or sustained reliability.
