"""Shared capability and bounded SDO evidence for generated/measured reports."""

SDO_RESOURCE_FIELDS = {
    "runtime_sdo_requests", "max_sdo_object_bytes", "sdo_object_storage_bytes"
}


def validate_sdo_resources(resources: dict, path: str) -> None:
    present = SDO_RESOURCE_FIELDS.intersection(resources)
    if not present:
        return  # Backward-compatible esop.product-build-input.v1 baseline.
    if present != SDO_RESOURCE_FIELDS:
        raise ValueError(f"{path} requires all three SDO capacity fields together")
    requests = resources["runtime_sdo_requests"]
    size = resources["max_sdo_object_bytes"]
    storage = resources["sdo_object_storage_bytes"]
    if type(requests) is not int or not 1 <= requests <= 64:
        raise ValueError(f"{path}.runtime_sdo_requests must be 1..64")
    if type(size) is not int or not 1 <= size <= 65536:
        raise ValueError(f"{path}.max_sdo_object_bytes must be 1..65536")
    if type(storage) is not int or storage != requests * size:
        raise ValueError(f"{path}.sdo_object_storage_bytes must equal requests * object bytes")


def validate_mailbox_capabilities(devices: dict, path: str) -> None:
    if "mailbox_capabilities" not in devices:
        return  # Legacy reports have no capability evidence; never infer CoE.
    entries = devices["mailbox_capabilities"]
    if not isinstance(entries, list) or len(entries) != devices["declared_slaves"]:
        raise ValueError(f"{path}.mailbox_capabilities must match declared_slaves")
    expected = devices["coe_complete_access"]
    information = devices["coe_sdo_information"]
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or set(entry) != {
            "name", "position", "capability", "pdo_configuration", "pdo_upload_supported"
        }:
            raise ValueError(f"{path}.mailbox_capabilities[{index}] has invalid fields")
        if (entry["name"], entry["position"]) != (expected[index]["name"], expected[index]["position"]):
            raise ValueError(f"{path}.mailbox_capabilities identities must match CoE records")
        if entry["capability"] not in ("none", "mailbox_only", "coe"):
            raise ValueError(f"{path}.mailbox_capabilities has unknown capability")
        if entry["pdo_configuration"] not in ("fixed", "assignment", "configurable"):
            raise ValueError(f"{path}.mailbox_capabilities has unknown PDO configuration")
        if type(entry["pdo_upload_supported"]) is not bool:
            raise ValueError(f"{path}.mailbox_capabilities PDO upload must be boolean")
        if entry["capability"] != "coe" and (
            entry["pdo_configuration"] != "fixed" or entry["pdo_upload_supported"]
            or expected[index]["supported"] or expected[index]["enabled"]
            or information[index]["supported"] or information[index]["enabled"]
        ):
            raise ValueError(f"{path}.mailbox_capabilities grants CoE features without CoE")
