use aws_sdk_cloudformation::{
    model::{Parameter, Stack, StackStatus},
    output::CreateStackOutput,
    Client,
};
use failure::Error;

fn lift_to_param(key: impl Into<String>, value: impl Into<String>) -> Parameter {
    Parameter::builder()
        .parameter_key(key)
        .parameter_value(value)
        .build()
}

async fn launch_stack(
    client: &Client,
    launch_template: &String,
    name: &String,
    instance_type: &String,
    port: &usize,
    key_name: &String,
    ssh_location: &String,
) -> Result<CreateStackOutput, Error> {
    // TODO tokio tracing, consider instrument
    println!("Launching instance...");
    println!("Instance Name: {}", name);
    println!("Instance type: {}", instance_type);
    println!("Socat Port: {}", port);
    println!("Key Name: {}", key_name);

    let stack = client
        .create_stack()
        .stack_name(name)
        .template_body(launch_template)
        .parameters(lift_to_param("InstanceName", name))
        .parameters(lift_to_param("InstanceType", instance_type))
        .parameters(lift_to_param("Port", port.to_string()))
        .parameters(lift_to_param("KeyName", key_name))
        .parameters(lift_to_param("SSHLocation", ssh_location));
    let stack_output = stack.send().await?;
    Ok(stack_output)
}

async fn get_stack(client: &Client, stack_id: &str) -> Result<Stack, Error> {
    let resp = client.describe_stacks().stack_name(stack_id).send().await?;
    let this_stack = resp.stacks().unwrap_or_default().first().unwrap();
    Ok(this_stack.clone())
}

async fn check_stack_status(
    client: &Client,
    stack_id: &str,
) -> Result<(StackStatus, String), Error> {
    let this_stack = get_stack(client, stack_id).await?;
    let stack_status = this_stack.stack_status().unwrap();
    let stack_status_reason = this_stack.stack_status_reason().unwrap_or("");
    Ok((stack_status.clone(), stack_status_reason.to_string()))
}

pub async fn launch(
    client: &Client,
    launch_template: &String,
    name: &String,
    instance_type: &String,
    port: &usize,
    key_name: &String,
    ssh_location: &String,
) -> Result<Vec<(String, String)>, Error> {
    let stack_output = launch_stack(
        client,
        launch_template,
        name,
        instance_type,
        port,
        key_name,
        ssh_location,
    )
    .await?;
    let stack_id = match stack_output.stack_id() {
        Some(x) => x,
        None => {
            return Err(failure::err_msg(
                "Missing `stack_id` in CreateStackOutput, please check CloudFormation \
                logs to determine the source of the error.",
            ))
        }
    };
    let (stack_status, stack_status_reason) = loop {
        let (status, status_reason) = check_stack_status(client, stack_id).await?;
        tokio::time::sleep(tokio::time::Duration::new(2, 0)).await;
        if status != StackStatus::CreateInProgress {
            break (status, status_reason);
        }
    };
    match stack_status {
        StackStatus::CreateComplete => {
            println!(
                "Successfully launched enclave with stack ID {:?}",
                stack_output.stack_id().unwrap()
            );
        }
        StackStatus::CreateFailed => {
            return Err(failure::err_msg(
                "Received CreateFailed status from CloudFormation stack, please check \
                AWS console or AWS logs for more information.",
            ))
        }
        other_status => {
            return Err(failure::err_msg(format!(
                "{:#?}: {}",
                other_status, stack_status_reason
            )))
        }
    }
    // Stack was created successfully, report outputs to stdout
    let this_stack = get_stack(client, stack_id).await?;
    // TODO handle missing outputs in this unwrap, maybe w/ warning instead of error?
    let outputs: Vec<(String, String)> = this_stack
        .outputs()
        .unwrap()
        .iter()
        .map(|o| {
            let k = o.output_key().unwrap().to_string();
            let v = o.output_value().unwrap().to_string();
            (k, v)
        })
        .collect();
    Ok(outputs)
}