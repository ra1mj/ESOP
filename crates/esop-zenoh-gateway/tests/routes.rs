use esop_zenoh_gateway::{
    KeySpace, MAX_ZENOH_KEY_BYTES, PayloadContract, RouteDirection, RouteError, RouteKind,
};

fn key(space: &KeySpace, kind: RouteKind) -> Vec<u8> {
    let mut buffer = [0; MAX_ZENOH_KEY_BYTES];
    let length = space.write_key(kind, &mut buffer).unwrap();
    buffer[..length].to_vec()
}

#[test]
fn builds_versioned_namespace_routes_and_preserves_robot_isolation() {
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    assert_eq!(
        key(&space, RouteKind::State),
        b"esop/fleet_a/robot_01/state"
    );
    assert_eq!(
        key(&space, RouteKind::Command),
        b"esop/fleet_a/robot_01/cmd"
    );
    assert_eq!(
        space
            .route_for_key(
                &key(&space, RouteKind::State),
                RouteDirection::Publish,
                PayloadContract::RobotState,
            )
            .unwrap(),
        RouteKind::State
    );

    let other = KeySpace::new(b"fleet_a", b"robot_02").unwrap();
    assert_eq!(
        other.route_for_key(
            &key(&space, RouteKind::State),
            RouteDirection::Publish,
            PayloadContract::RobotState,
        ),
        Err(RouteError::UnknownRoute)
    );
}

#[test]
fn denies_wrong_direction_and_payload_contract() {
    let space = KeySpace::new(b"fleet_a", b"robot_01").unwrap();
    let command = key(&space, RouteKind::Command);
    assert_eq!(
        space.route_for_key(
            &command,
            RouteDirection::Publish,
            PayloadContract::MotionCommand,
        ),
        Err(RouteError::DirectionDenied)
    );
    assert_eq!(
        space.route_for_key(
            &command,
            RouteDirection::Subscribe,
            PayloadContract::RobotState,
        ),
        Err(RouteError::PayloadMismatch)
    );
    assert_eq!(
        space.route_for_key(
            &key(&space, RouteKind::Query),
            RouteDirection::Publish,
            PayloadContract::Query,
        ),
        Ok(RouteKind::Query)
    );
}

#[test]
fn validates_identifier_key_capacity_and_payload_bounds() {
    assert_eq!(
        KeySpace::new(b"", b"robot"),
        Err(RouteError::EmptyIdentifier)
    );
    assert_eq!(
        KeySpace::new(b"fleet/unsafe", b"robot"),
        Err(RouteError::InvalidIdentifier)
    );
    let space = KeySpace::new(b"fleet", b"robot").unwrap();
    let mut short = [0; 4];
    assert_eq!(
        space.write_key(RouteKind::State, &mut short),
        Err(RouteError::KeyBufferTooSmall)
    );
    assert_eq!(
        KeySpace::validate_payload(&[]),
        Err(RouteError::EmptyPayload)
    );
    assert_eq!(
        KeySpace::validate_payload(&[0; 4097]),
        Err(RouteError::PayloadTooLarge)
    );
    assert_eq!(KeySpace::validate_payload(&[1, 2, 3]), Ok(()));
}
